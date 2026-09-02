## Unreleased

### Added

- Guest-requested plugin loading: the `omnia:plugins/loader` host capability.
  A guest whose world imports it can ask the host to load a component at run
  time — `load(package, location, digest?)` returns a plain `plugin` record
  (`id`, `digest`), a value carrying no lifecycle authority over the loaded
  component; component bytes never cross the interface in either
  direction. The host pipeline is trust-ordered: idempotency on
  (package, digest) → acquisition through the deployment's compiled-in
  acquirer → sha256 pin verification (before any wasmtime validation;
  unpinned loads report the resolved digest for trust-on-first-use) → typed
  refusal of native/pre-compiled bytes → safe `GuestArtifact::wasm`
  validation only → refusal unless the component exports a declared plugin
  interface → registration under the package identity. Refusals are typed
  by the caller's remedy (`refused` for a wrong request or deployment,
  `unavailable` for a retryable acquisition failure, `already-active` for an
  identity conflict, `internal` for a host fault), with the description
  naming the specific cause; a deployment guest's identity can never be
  re-bound, and a conflicting re-pin of an active package refuses.
  Acquisition policy is a pair of
  composition-root slots, one per location kind (`RegistrySource`,
  `PathSource`), installed through `Plugins::install` by
  the `runtime!` macro's generated `Wiring::extend` hook from the declarative
  `plugins: { locations: [...] }` list; loads route structurally by kind and
  an empty slot refuses typed. The built-in acquirers ship
  in `omnia-plugin` and re-export: `PathMounts` (named directory roots
  opened fail-fast at startup, read fresh on every load) and
  `RegistryClient` (exact `namespace:name@version` references
  from a compiled-in default registry endpoint, verified against the
  registry's content digest, fresh-release-preferred when the `cache:`
  backend attaches a store — `ContentStore` for digest-keyed
  bytes, `ReleaseStore` for per-registry release records). The loader
  links once on the shared linker when a deployment declares `plugins`;
  wasmtime wires it only
  into guests whose world imports `omnia:plugins/loader`. See
  [docs/security-model.md](docs/security-model.md#guest-requested-plugin-loading-omniapluginsloader).
- The requester surface for the loader, in `omnia-guest`'s new `plugins`
  module: a `Plugins` capability trait (WASI-backed default body on `wasm32`
  over the crate's own `omnia:plugins/loader` bindings, bare natively so
  suites script loads), the `WasiPlugins` zero-sized provider, shared
  `PluginRef`/`Digest`/`Location` types compiled on both targets (`Digest`
  validates and canonicalizes `sha256:<hex>` pins, with serde support), a
  `Plugin` handle carrying the routed identity and resolved digest, typed
  refusals mirroring the WIT error variant with kebab-case `code()`
  discriminants and a conversion into the guest `Error` taxonomy
  (`unavailable` → `BadGateway`, `internal` → `ServerError`, every other
  refusal → `BadRequest`), and `PluginCache` — ensure-once memoization of
  handles by package identity (never bytes; a conflicting re-pin refuses
  `already-active`, mirroring the host). No consumer vocabulary anywhere:
  any requester-class world can use it.

- `hosts:` rows accept compiled-in connect options: `Host: Backend(options)`
  lowers to `Backend::connect_with(options)` instead of the env-sourced
  `Backend::connect()`. Use it to compile configuration into the binary — a
  fixed storage root (e.g. a project CAS path), or a scripted test backend
  carrying state without statics. Rows sharing a backend type share one
  connection, so their options must be written identically on every row (or
  omitted on every row) — a mismatch is a spanned compile error, as are
  empty parentheses.

  ```rust
  hosts: {
      WasiKeyValue: Filesystem(FilesystemOptions::at(".omnia/storage")),
      WasiBlobstore: Filesystem(FilesystemOptions::at(".omnia/storage")),
  }
  ```

- `omnia-test` (`crates/test`, unpublished): test doubles, a component
  runtime harness, and a `wasm32-wasip2` fixture pipeline behind three
  additive features. `guest` (default) carries a native double per
  `omnia_guest` capability trait — `Scripted: Model`, `ScriptedLoader:
  Plugins`, `Memory: StateStore + BlobStore` (with `Namespaced`),
  `MemoryDocs` over the docstore default, `ScriptedTables`, `MatchedHttp`,
  `Sink: Publish + Broadcast`, `MapConfig`, `FixedIdentity` — plus
  `doubles!` (a `provider!`-shaped declaration seeded with the default double
  per capability) and `forward!` (delegating capability impls to fields, with
  a bracketed generic header). `host` carries `Deployment`, a manifest-driven
  command run over `Backends`, the twelve in-memory defaults with the model
  swappable for a `ScriptedModel: WasiModelCtx`, and `Scratch` for per-test
  directories. `build` is `std`-only: `Components` runs the nested cargo
  build a consumer's `build.rs` needs and writes `gen.rs` with one path
  constant per program and a `foreach_<group>!` completeness macro per
  group. Both scripted models share one `Script` core, so the same script
  reads identically at the handler and component rungs.

### Changed

- Plugin loads are lock-free and race-safe. The loader's (package, digest)
  idempotency record now rides the registry entry itself (`Guest::digest`,
  recorded by `Runtime::admit` from the admitted bytes), so the attestation
  can never outlive or misdescribe the guest it names — an identity
  deregistered and re-registered by the embedder no longer answers a pinned
  re-load with a stale digest. The loader's shadow digest map and the global
  load mutex are deleted; concurrent loads race through `admit`, whose
  atomic publication reports the loser via the new
  `AdmitError::AlreadyRegistered` variant, resolved against the winner's
  recorded digest (idempotent success on a match, `already-active`
  otherwise). `sha256_digest` moves to `omnia-core` (still re-exported from
  `omnia`).
- Acquirers refuse honestly: `RegistrySource` and `PathSource` return the
  loader's typed `LoadError` — `refused` for an authoritative "no" (a
  malformed reference, a package or path the source does not serve),
  `unavailable` for a failure a retry may clear — so an unknown package no
  longer reports as retryable.
- The runtime core drops "plugin" from its vocabulary: the manifest accessor
  `Manifest::plugin_interfaces()` is renamed to `link_interfaces()`, and
  admission refusals name "link interfaces". The config surface is
  unchanged — the TOML `plugins = [...]` list, the macro `plugins:` block,
  and the CLI `--plugins` flag keep their names; only `omnia-plugin` speaks
  "plugin".
- The `omnia` crate splits into `omnia-core` (the runtime spine: deployment,
  registry, dispatch, stores, telemetry, CLI) and a thin `omnia` facade that
  re-exports core, `omnia-plugin`, and the `runtime!` macro under the
  existing paths — embedder imports are unchanged. `omnia-plugin` is now the
  whole plugins capability: the loader WIT and `WasiPlugins` host binding,
  the `Plugins` load path, digest policy, and the acquisition seam all
  live there, built on two intentional core seams that future capability
  crates reuse: `Runtime::admit` (the one privileged operation — safe
  validation, seam-export check, atomic registration; typed `AdmitError`)
  and the `Extensions` typemap (capability state installed by the new
  `Wiring::extend` hook — replacing `Wiring::acquirer` — and read back from
  stores through `HasExtensions`; state that calls back into the runtime
  holds a `WeakRuntime` via `Runtime::downgrade`). `Runtime::new`'s third
  parameter is now the extend hook (`FnOnce(&Runtime<B>) -> Result<()>`);
  `StoreConfig::loader`/`StoreBase::loader` are replaced by the `extensions`
  handle, and the loader's digest record rides the registry entry itself
  (`Guest::digest`, recorded at admission), living and dying with the guest
  it attests.
- Host wiring is trait-carried, not name-derived (pre-1.0 hard cut, no
  aliases). Omnia core gains `HostCtx` (the host's borrow shape and view
  assembly, `Borrow<'a>` GAT + `view`), `Provides<H>` (the one
  bundle-accessor trait), and `StoreView<H>` (the one store-side view trait,
  blanketed on `StoreCtx<B>` for every `B: Provides<H>`). The per-host
  `Wasi*View` traits, `Has*` accessor traits, and per-host `StoreCtx`
  blankets are deleted; `wasi_view!` now emits the `CtxView` struct plus the
  host's `HostCtx` impl, and host `add_to_linker` accessors ride
  `T: StoreView<WasiX>` / `T::view`. The `runtime!` macro emits one uniform
  `impl omnia::Provides<...> for Backends` per `hosts:` row — no more name
  surgery from the host ident, so re-exports and third-party hosts wire the
  same way as first-party ones, and `wasi:config`'s shared-borrow shape is
  carried by its own `HostCtx` impl instead of a codegen special case. One
  special case survives, documented in codegen: `wasi:http`'s view trait is
  foreign (`wasmtime-wasi-http`), so its row is keyed to the core-owned
  `omnia::HttpCtx` carrier (`HasHttp` is replaced by the backend-level
  `HttpBorrow` trait), and a dodged match now fails at compile time rather
  than silently. Hand-built bundles implement `Provides<WasiX>` directly.

  ```rust
  // before                              // after
  impl HasModel for Backends {           impl omnia::Provides<WasiModel> for Backends {
      fn model_ctx(&mut self)                fn borrow(&mut self)
          -> &mut dyn WasiModelCtx {             -> &mut dyn WasiModelCtx {
          &mut self.0                            &mut self.0
      }                                      }
  }                                      }
  ```
- The `runtime!` macro serves every `hosts:` row uniformly through
  `Server::run`: capability hosts resolve immediately via the no-op default,
  trigger servers loop until shutdown. The macro's string-matched server
  list and the unread `Server::IS_SERVER` const are deleted; a third-party
  trigger host's `run` is now actually served instead of silently skipped.
- The `runtime!` macro's `plugins:` key grows from a bracketed list to a
  block: `plugins: { interfaces: [...], locations: [...], cache: ... }`. The
  declarative `locations:` list is the acquisition policy — named path roots
  and at most one registry endpoint, lowered into the built-in
  `PathMounts`/`RegistryClient` acquirers slotted by location kind —
  and the optional `cache:` names the backend that joins the generated
  bundle as the registry's store. Both keys are optional (a
  deployment that only does host-mediated dispatch needs no acquirer; loads
  then refuse typed at run time); an empty `locations:` list, a second
  `registry` entry, and a `cache:` without a registry entry are spanned
  compile errors. The bare-list form is a compile error naming the block
  shape. Declaring the block also links the `omnia::WasiPlugins` loader
  host. Because acquisition policy is compiled-in code rather than manifest
  data, a policy-only block composes with `config:` — the TOML declares the
  interfaces, the binary the acquirer. TOML manifests are unchanged
  (`plugins = [...]` stays a plain interface list; an acquirer cannot ride a
  config file).

  ```rust
  // before                              // after
  plugins: ["omnia:link/echo"],          plugins: {
                                             interfaces: ["omnia:link/echo"],
                                             locations: [        // optional
                                                 { name: ".", path: "." },
                                                 { registry: "ghcr.io" },
                                             ],
                                             cache: Filesystem,  // optional
                                         },
  ```
- The host-mediated interface list is renamed from `dispatch` to `plugins`:
  a top-level `plugins = [...]` in TOML, a top-level `plugins: [...]` key in
  the `runtime!` macro, the fluent `Manifest::plugins(...)` setter (with the
  `Manifest.plugins` field and the `Manifest::link_interfaces()` accessor),
  and the CLI flag `--plugins` (replacing `--dispatch`, no alias). Stale keys
  fail loudly: a leftover top-level `dispatch` (or `link`) is a parse/compile
  error, and `plugins` misplaced on a guest entry is rejected with a pointed
  diagnostic. Behavior is unchanged: listed interfaces are polyfilled onto
  the shared linker at assemble (an exporter may arrive later via
  `Runtime::register`), and the selector still picks the target guest by
  routing id at call time. The dispatch *mechanism* keeps its names —
  `crates/omnia/src/dispatch`, `serve_links`, `DispatchHandle`,
  `MAX_DISPATCH_DEPTH`, and the `omnia:link` WIT packages are untouched.

  ```toml
  # before                            # after
  dispatch = ["omnia:link/echo"]      plugins = ["omnia:link/echo"]
  ```
- The host-mediated interface list is renamed from `link` to `dispatch`
  and is deployment-wide only: a top-level `dispatch = [...]` in TOML, a
  top-level `dispatch: [...]` key in the `runtime!` macro, the fluent
  `Manifest::dispatch(...)` setter, and the CLI flag `--dispatch` (replacing
  `--link`, no alias). The per-guest form (`GuestEntry.link` in TOML,
  `link:` on a macro guest entry, `GuestEntry::link()`) is removed — the
  linker is shared, so per-guest lists always flattened into one
  deployment-level grant and never enforced a per-guest ACL. Stale keys
  fail loudly: a leftover `link` (top-level or per-guest) or a `dispatch`
  misplaced on a guest entry is a parse/compile error. Behavior is
  unchanged: listed interfaces are polyfilled onto the shared linker at
  assemble (an exporter may arrive later via `Runtime::register`), and the
  selector still picks the target guest by routing id at call time. WIT
  packages (`omnia:link`) and internals (`serve_links`,
  `DispatchHandle::links`) keep their names.

  ```toml
  # before                            # after
  [[guest]]                           dispatch = ["omnia:link/echo"]
  id = "router"
  source.path = "./router.wasm"       [[guest]]
  link = ["omnia:link/echo"]          id = "router"
                                      source.path = "./router.wasm"
  ```
- Routes are now guest-owned: each `[[guest]]` entry declares the routes
  targeting it (`routes.http` / `routes.messaging` / `routes.websocket`
  pattern lists in TOML, a `routes: { http: [...], ... }` block per guest
  entry in the `runtime!` macro, `GuestEntry::route_http` /
  `route_messaging` / `route_websocket` programmatically), with the
  declaring guest as the implicit target. The top-level `[[route.*]]`
  tables, the macro's top-level `routes:` key, the `Manifest::route_*`
  setters, and the `RouteSpec` / `HttpRoute` / `TopicRoute` types are
  removed; a stale top-level route section now fails manifest parsing.
  Routing behavior is unchanged: per-guest lists aggregate into the same
  per-trigger tables (longest-prefix HTTP, first-match NATS-style
  patterns, capability catch-all when a trigger has no routes).

  ```toml
  # before                            # after
  [[guest]]                           [[guest]]
  id = "api"                          id = "api"
  source.path = "./api.wasm"          source.path = "./api.wasm"
                                      routes.http = ["/api"]
  [[route.http]]
  prefix = "/api"
  guest = "api"
  ```
- Removed the `runtime!` macro's `program:` key: `mode: command` with a
  compiled-in deployment (`config:` or inline manifest keys) is now a
  direct command by default — raw argv passthrough with the reserved
  `--debug` / `--quiet` host log flags, no host `run` grammar. The program
  name (telemetry and guest `argv[0]`) defaults to the manifest name (first
  `[[guest]]` id). Command-mode binaries without a compiled-in deployment
  keep the `run` grammar
- Removed the `command_guest:` key and its plumbing
  (`DeploymentBuilder::command_guest`, `Runtime::with_command_guest`,
  `MainOptions::command_guest`): command mode routes to the sole static
  `wasi:cli/run` exporter, or to the guest entry marked `command = true`
  (macro `command: true`, `GuestEntry::command()`); at most one guest may
  carry the mark. The resolver-supplied command guest (fully dynamic
  deployment, empty guest set) is gone with the key — a direct command
  always compiles its manifest in
- Removed the late-binding deployment plumbing: the resolve-on-miss pull
  layer (`GuestResolver`, `Runtime::ensure_guest`, the single-flight
  machinery, macro `resolver:`, `DeploymentBuilder::resolver`,
  `Runtime::with_resolver`), the `http_paths` trigger hook (macro
  `http_paths:`, `DeploymentBuilder::http_paths`,
  `Runtime::with_http_paths`, `RoutingPolicy` / table-only routing,
  `Runtime::route_http`, `RouteRefusal`), and pre-bound HTTP listener
  adoption (macro `http_listener:`, `DeploymentBuilder::http_listener`,
  `Runtime::take_http_listener`). A registry miss is a dispatch error and
  an unrouted HTTP path is a 404; the HTTP trigger always binds
  `HTTP_ADDR`. Push registration stays: `Runtime::register` / `deregister`
  and `DeploymentBuilder::dynamic()` are the way a registry grows after
  boot, with registered guests reachable via host-mediated link dispatch
  and `Dispatcher::invoke`

## 0.35.0

Released 2026-07-25

Paired production-backends release: [omnia-backends 0.29.x](https://github.com/augentic/omnia-backends/blob/main/RELEASES.md).

### Added

- Multi-guest registry: one process hosts many Wasm components on a shared
  engine/linker, selected by opaque `GuestId`, with instance-per-call
  instantiation, route tables, and host-mediated guest-to-guest linking
- Resolve-on-miss `GuestResolver` so missing guests can be faulted in at
  dispatch time (HTTP fallback, command routing, single-flight per identity)
- `runtime!` keys for resolver-backed deployments: `resolver:`,
  `command_guest:`, `program:`, and compile-time `config:` fallback
- Embedded guest bytes (`include_bytes!` / `Source::embedded`) for
  single-binary hosts alongside path-sourced guests
- Dynamic `Runtime::register` / `deregister` after bootstrap, including late
  import polyfill for guests admitted after static assembly
- Pooling allocator enabled by default, with environment-driven
  `RuntimeOptions` shared by `omnia compile` and `omnia create`
- `omnia-wasi-model` host interface and workspace packaging for model /
  working-tree workflows
- Stateless MCP stack in `omnia-guest` (JSON-RPC + Streamable HTTP) and
  testkit helpers for MCP grant recording
- Concurrent (`async func`) host-mediated link dispatch via
  `func_new_concurrent` / `send_concurrent`
- `omnia-testkit` (dev-only) and an integration-first testing posture

### Changed

- `omnia-sdk` renamed to `omnia-guest`; host/guest macro crates split into
  `omnia-host-macros` and `omnia-guest-macros`; `omnia-otel` folded into
  `omnia`; `omnia-wasi-jsondb` replaced by `omnia-wasi-docstore`
- Deployments center on `Manifest` / `omnia.toml` (`OMNIA_CONFIG`), with
  `[[mount]]` working-tree preopens and typed `DeploymentBuilder` paths
  (safe wasm vs trusted precompiled)
- Upgraded `wasmtime` to 47.0.2 (and matching `wasmtime-wasi*`),
  `wit-bindgen` to 0.60, `wasip3` to 0.7, and `cap-std` / `cap-fs-ext` to 4.x
- Compile-affecting runtime toggles now include `DEBUG_SYMBOLS` and
  `GENERATE_ADDRESS_MAP` so AOT compile and load stay aligned

<!-- Release notes generated using configuration in .github/release.yaml at main -->

## What's Changed
* Bump to 0.35.0 by @augentic-releases[bot] in https://github.com/augentic/omnia/pull/202
* Instrumentation fix by @andrewweston in https://github.com/augentic/omnia/pull/203
* Instance pooling by @andrewweston in https://github.com/augentic/omnia/pull/204
* Guest registry by @andrewweston in https://github.com/augentic/omnia/pull/205
* Implement wasi model by @andrewweston in https://github.com/augentic/omnia/pull/206
* Specify readiness testing by @andrewweston in https://github.com/augentic/omnia/pull/207
* MCP server for cursor-agent by @andrewweston in https://github.com/augentic/omnia/pull/208
* Post-upgrade testing and code review by @andrewweston in https://github.com/augentic/omnia/pull/209
* Async guest-2-guest linking by @andrewweston in https://github.com/augentic/omnia/pull/210
* style fenced code by @andrewweston in https://github.com/augentic/omnia/pull/211
* Specify-driven refactoring by @andrewweston in https://github.com/augentic/omnia/pull/212
* Streamline testing by @andrewweston in https://github.com/augentic/omnia/pull/213
* Replay by @andrewweston in https://github.com/augentic/omnia/pull/214
* MCP grants by @andrewweston in https://github.com/augentic/omnia/pull/215
* Runtime flexibility by @andrewweston in https://github.com/augentic/omnia/pull/216
* improve runtime config by @andrewweston in https://github.com/augentic/omnia/pull/217
* Dynamic guest resolver by @andrewweston in https://github.com/augentic/omnia/pull/218
* Dynamic guest resolver in runtime! by @andrewweston in https://github.com/augentic/omnia/pull/219
* Embed guest bytes by @andrewweston in https://github.com/augentic/omnia/pull/220
* Update to wasmtime 47.0.2 by @andrewweston in https://github.com/augentic/omnia/pull/221

**Full Changelog**: https://github.com/augentic/omnia/compare/v0.34.0...v0.35.0

---

Release notes for previous releases can be found on the respective release branches of the repository.

<!-- ARCHIVE_START -->
* [0.35.x](https://github.com/augentic/omnia/blob/release-0.35.0/RELEASES.md)
* [0.34.x](https://github.com/augentic/omnia/blob/release-0.34.0/RELEASES.md)
* [0.33.x](https://github.com/augentic/omnia/blob/release-0.33.0/RELEASES.md)
* [0.32.x](https://github.com/augentic/omnia/blob/release-0.32.0/RELEASES.md)

- [0.31.x](https://github.com/augentic/omnia/blob/release-0.31.0/RELEASES.md)
- [0.30.x](https://github.com/augentic/omnia/blob/release-0.30.0/RELEASES.md)
- [0.29.x](https://github.com/augentic/omnia/blob/release-0.29.0/RELEASES.md)
- [0.28.x](https://github.com/augentic/omnia/blob/release-0.28.0/RELEASES.md)
- [0.27.x](https://github.com/augentic/omnia/blob/release-0.27.0/RELEASES.md)
- [0.25.x](https://github.com/augentic/omnia/blob/release-0.25.0/RELEASES.md)
- [0.23.x](https://github.com/augentic/omnia/blob/release-0.23.0/RELEASES.md)
- [0.22.x](https://github.com/augentic/omnia/blob/release-0.22.0/RELEASES.md)
- [0.21.x](https://github.com/augentic/omnia/blob/release-0.21.0/RELEASES.md)
- [0.20.x](https://github.com/augentic/omnia/blob/release-0.20.0/RELEASES.md)
- [0.20.x](https://github.com/augentic/omnia/blob/release-0.20.0/RELEASES.md)
- [0.19.x](https://github.com/augentic/omnia/blob/release-0.19.0/RELEASES.md)
- [0.18.x](https://github.com/augentic/omnia/blob/release-0.18.0/RELEASES.md)
- [0.17.x](https://github.com/augentic/omnia/blob/release-0.17.0/RELEASES.md)
- [0.16.x](https://github.com/augentic/omnia/blob/release-0.16.0/RELEASES.md)
- [0.15.x](https://github.com/augentic/omnia/blob/release-0.15.0/RELEASES.md)
