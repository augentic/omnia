## 0.37.0

Unreleased

### Added

- One tracing level for the whole process, selected by verbosity flags. A
  run's level is the flag, else the process `RUST_LOG`, else the mode's
  default — `info` for a command, `warn` for a server (`Mode::level`) — and
  it governs the host console and every guest alike: each guest's WASI
  environment carries it as `RUST_LOG` (a selected level replaces the
  process variable there; a bare run fills it only when unset), and the
  process environment is never written. Each `-v`/`--verbose` steps the
  level one rung up the scale `off`, `error`, `warn`, `info`, `debug`,
  `trace` from the mode's default and each `-q`/`--quiet` one rung down,
  clamped at the ends, so a command reads `-q` warn, `-qq` error, `-v`
  debug, `-vv` trace, and a server `-q` error, `-qq` off, `-v` info, `-vv`
  debug, `-vvv` trace. On the `run` grammar the flags are global to the
  command (`bin -v run …`, `bin run -v …`; `-v` beside `-q` is a usage
  error); on the direct-command path the host reads them out of argv before
  `--` and forwards argv verbatim, so a command guest declares the same flags
  by flattening `omnia_sdk::api::command::Verbosity` into its grammar, which
  lists them in help and completions, accepts them, and refuses the pair with
  its own usage error — the guest never acts on the values. Embedders select
  the level in code with `DeploymentBuilder::level(LevelFilter)` (`omnia`
  re-exports `LevelFilter`), `Telemetry::fallback(level)` sets the console's
  fallback for an unset `RUST_LOG` (`WARN` when not called, as before), and
  the test host's `Deployment::level` scripts a guest's `RUST_LOG` without
  touching the suite's environment. A bare command run's host console now
  opens at `info`, where it opened at `warn`.
- The dispatch-chain context lives on the guest store. Every store is built
  at a `ChainCtx` (`StoreConfig::chain`, `StoreBase::chain`):
  `Runtime::store()` builds a server root, `Runtime::store_in(chain)` any
  other, and a `StoreFactory` takes the context its callee runs at, so the
  command driver, the trigger hosts, and `call_fresh` carry no ambient
  scope. The link relay reads the calling guest's context from its store
  through `HasChain` (implemented by `StoreCtx` beside `HasMounts` and
  `HasExtensions`), `ChainPolicy::enter(&caller, &target)` derives the
  callee's, and `Dispatcher::invoke` takes the caller's context ahead of
  the target.

### Changed

- One loader for either artifact format, everywhere a guest loads. The
  bytes a declared guest resolves to — a manifest `source.path`, the
  macro's compiled-in `path:`, a loader entry's file or registry package —
  are deserialized when they are `omnia compile` output and compiled when
  they are raw wasm, told apart the way wasmtime tells them apart
  (`Engine::detect_precompiled`); a settings-mismatched pre-compiled
  artifact still fails with the compile-settings hint. What the deployment
  names is the operator's trusted input in either format
  (`docs/security-model.md`), so the raw-wasm/pre-compiled trust split is
  gone with its API: `DeploymentBuilder::build_trusted` (`build` is the one
  build, and the generated `run` / `run_with` load a pre-compiled guest as
  `main` does), `GuestArtifact`, `is_precompiled`, and `ELF_MAGIC`. Two
  paths stay raw-wasm-only, because their bytes are not the deployment's
  word: `Runtime::register(id, bytes)`, the embedder's untrusted-bytes path,
  refuses a pre-compiled artifact (an artifact the embedder's own build
  produced goes through `Runtime::admit` wrapped in the `unsafe`
  `Verified::trusted`), and an on-demand guest — read while guests already
  run — admits one only when its entry pins the `digest` or embeds the
  bytes, refusing an unpinned `source.path` or `source.package` that
  resolves to one (`refused`, which the `omnia:plugins/loader` WIT names).
  - Native code is admitted through one token. `omnia::Verified` is
    component bytes with their digest, obtained from exactly three places:
    `Source::verified`, once the pin, the `wasm_only` mark, and the
    on-demand rule have passed; `Verified::wasm`, which admits raw wasm
    alone; and the `unsafe` `Verified::trusted`, the embedder's word for an
    artifact of its own. `Runtime::admit(id, verified)` takes it (where it
    took bytes and a digest), `register` delegates through `Verified::wasm`,
    and the loader hands over the token it verified rather than hashing
    twice.
  - The digest is never optional. Every path a guest's bytes take runs
    through one body, `Runtime::admit`, and every registration records
    the digest of the bytes it was loaded from: `Guest::digest` and
    `LoadedGuest::digest` are a `Digest`, `Plugin::digest` is a `Digest` on
    the host and a `&Digest` in the SDK (the WIT record's `digest` is a
    `string`), and a load of an active guest always attests it.
    `Guest::with_digest` and the test loader's `ScriptedLoader::unhashed`
    are gone with the `None` they scripted.
  - One `Source` for boot and on-demand guests. `omnia::Source` (in
    `omnia-core`, beside the `SourceSpec` that moved there under the same
    `omnia::SourceSpec` path) is a `[[guest]]` entry resolved for loading —
    identity, `SourceSpec`, digest pin, `wasm_only`, and the `on_demand`
    mark for an entry read at first load — with `read`, `verified`, and
    `load`, so the pin check and the format policy are one body at boot and
    on demand. `Manifest::boot_sources()` and `Manifest::on_demand_sources()`
    are the two filters over it (`sources()` and `on_demand()` are renamed;
    the latter marks each `Source::on_demand()`, as `Plugins::install` does
    for any it is handed), `Plugins::install` takes the on-demand `Source`s,
    and omnia-plugin's `Origin` and `OnDemand` are gone.
  - `wasm_only`, a per-entry policy for sources the deployment declares
    from input it does not author: `GuestEntry::wasm_only()` (TOML
    `wasm_only = true`, the macro's `wasm_only: true`) refuses a
    pre-compiled artifact however it hashes, at boot (assembly fails) or on
    demand (`refused`, which the WIT now names). The pin proves the bytes
    are the ones the entry named; this proves they run inside the sandbox.
  - The compile-affecting settings are one explicit value. `CompileOptions`
    (re-exported from `omnia`) holds the six — `Default` is the environment
    defaults, `RuntimeOptions::compile_options()` the loaded environment's —
    and `CompileOptions::configure(&mut Config)` is the one body the runtime
    engine and the compiler apply them through.
    `omnia::compile::compile(wasm, output, target, &CompileOptions)` takes
    them and the target triple explicitly instead of reading its own
    environment and host, so a `build.rs` compile is steered by neither the
    build shell nor the build machine; the CLI's `compile` gains
    `-t, --target <triple>`.
- Host telemetry is no longer feature-gated. The `otlp` feature is gone from
  `omnia` and `omnia-core`: every build of `Telemetry` carries the OTLP span
  and metric exporters beneath the console subscriber, so
  `omnia::telemetry::flush`, `omnia::telemetry::resource`, and
  `OTEL_GRPC_URL` are never compiled out, and a `default-features = false`
  build of `omnia` exports too. Whether they take effect at run time is
  unchanged from 0.36.0: `Telemetry::build` publishes the providers when it
  installs the subscriber, and yields (leaving `flush` a no-op and
  `resource` `None`) when an embedder's own subscriber is already set.
  `Telemetry::{new, endpoint, filter, build}` are as in 0.36.0; `fallback`
  is new (above). A later `build` after an embedder's own subscriber is
  settled without retrying or re-warning. The OpenTelemetry crate family is
  0.33 (`tracing-opentelemetry` 0.34).
- `ChainCtx` is no longer `Default`: a root is `ChainCtx::server()` or
  `ChainCtx::command()`. `as_command_chain(fut)` is now a store built at the
  command root, `runtime.build_store(runtime.store_in(ChainCtx::command()))`,
  and `Dispatcher::invoke` takes the caller's `ChainCtx` before the target.
- Nothing declares what guests call between themselves; the host tells its
  own interfaces from the ones guests share by namespace.
  - An import or export under `wasi:` or `omnia:` is the host's; every other
    interface a guest imports is relayed to whichever guest exports it.
    `omnia::is_host` is the predicate, and it is closed: a native host
    linked under any other namespace collides with the relay at boot.
  - The `services` list is gone from the manifest, the `runtime!` grammar,
    the test deployment, and the CLI (`run --link` with it);
    `InProcessLinks::new(selector, policy)` takes no interface list.
  - An import no guest exports fails at boot; a dispatch to a guest exporting
    no linked interface fails at the call site.
  - A guest's own package (`example:link`, `emery:adapter`) never collides
    with the host's — the `guest-link` example's WIT moved from `omnia:link`
    to `example:link`.
  - The `link` cargo feature keeps its name and now governs only whether
    such imports are relayed at all.
- The `runtime!` grammar embeds its guests.
  - `guests:` is a list, `guests: [ { path | package, name?, routes?,
    command?, on_demand?, digest? }, .. ]`.
    Each entry's `path:` is a string literal or a macro expanding to one
    (`concat!(env!(..), ..)`, a `build.rs`-emitted `env!("GUEST_WASM")`) that
    the macro reads with `include_bytes!` — the component is compiled into
    the binary, never read from disk at start; a component read at start is
    a `manifest:` file's `[[guest]]`. A computed `path:` is a compile error.
  - A guest is named by the path's file stem unless `name:` says otherwise.
  - `registries:` is a root key beside `guests:` and `mounts:`, the routing
    of the deployment's on-demand `package:` guests.
  - The generated `main` passes the invoking crate's `CARGO_PKG_NAME` as the
    program name — telemetry's component name and, in command mode, the
    guest's `argv[0]` — where it was the first guest's identity;
    `DeploymentBuilder::program_name` sets it on a programmatic deployment,
    and one that sets none is `omnia`. `Manifest::name` is gone.
  - The repository's `examples` package compiles the guests `cli-static` and
    `guest-link` embed in a `build.rs` through
    `omnia_test::build::Components`, so those run with no guest build first.
- A guest entry's identity is its `name`.
  - `[[guest]] name = ".."` replaces `id`; it may be omitted, in which case
    the `source.path` file's stem names the guest (`./guests/echo.wasm` is
    `echo`, `Manifest::load` derives it). An entry nothing names — a
    `package` source without `name` — is refused at startup beside a
    duplicate name.
    The retired `id` key is an unknown key.
  - `GuestEntry::new(name, source)`, `GuestEntry::embedded(path, bytes)`
    (named by the path's stem; what the macro lowers to),
    `GuestId::from_path`; the test host's `Deployment` takes
    `guest(name, ..)` and `command(name)`.
- The `link:` and `plugin:` blocks, `omnia::Location`, `LinkConfig`, and
  `PluginConfig` are gone; `config:` is `manifest:`.
  - The TOML manifest follows: `[registries] path = "wasm-pkg.toml"` replaces
    `[[plugin.location]]`, and `Manifest::load` replaces
    `Manifest::from_config`. `omnia_test::host::Deployment::path_root` is
    gone with the location list, and the test deployment's `locations`
    builder is `registries`.
  - The guest loader is assembly's, not the macro's: `Deployment::assemble`
    links `WasiPlugins` beside WASI whenever omnia is built with the feature
    and installs the manifest's on-demand guests (below) through
    `Plugins::install(runtime, on_demand, registry)`; `Wiring::extend` and
    `Deployment::plugin_locations` are removed, and the generated `Hooks`
    carry `link` and `serve` alone.
  - `RegistryClient::new` takes the wasm-pkg `Config` (there is no
    `with_config`), `RegistryClient::from_toml` the deployment's `registries`
    contents, and one is always installed, over an empty configuration when
    the deployment declares none; `Deployment::registry_source(..)` selects a
    custom `RegistrySource` — a `RegistryClient::cached` over a
    `ContentStore` + `ReleaseStore`, say — before assembly. A package the
    configuration routes nowhere (no `default_registry`, no mapping for its
    namespace) is refused typed, naming the namespace, before any registry
    is dialled.
  - The `plugin` cargo feature is `loader`.
  - On the CLI, `--config`/`-c` is `--manifest`/`-m`, `OMNIA_CONFIG` is
    `OMNIA_MANIFEST`, and `--link` is gone; `RunSource::Config` is
    `RunSource::Manifest`.
- The deployment's grant bounds every load: a guest names a location, and
  the host resolves it inside what the deployment declared.
  - `omnia:plugins/loader.load(from: location, digest: option<string>)`
    returns the `plugin` handle (`id`, `digest`). The `location` gains a
    third arm, `declared(name)`, naming one of the deployment's guests; the
    `registry` arm carries a `registry-ref { package, endpoint? }` and the
    `path` arm a component path, so the separate `package` parameter is
    gone — a load registers under the name its location derives: the
    declared name, the path's file stem, or the package reference. A
    `declared` name outside the deployment's table is refused typed (where
    the same name dispatched blind would trap), one already active is
    attested without fetching anything, and it takes no digest of its own —
    the entry carries the pin. The `already-active` variant is gone: a
    location whose name is active under other bytes is `refused`, and the
    same bytes attest. A `path` or `registry` whose derived name the
    deployment declares is `refused` before anything is read: a declared
    name is bound by its entry alone.
  - A component loads from a read-only mount alone. A `path` resolves to
    the mount it lies beneath — `.` for a bare relative path, a mount's name
    as its prefix otherwise — and is refused before the file is read when
    that mount is writable, so a guest cannot plant a component through a
    writable mount and have the host compile it; `Deployment::assemble`
    refuses a writable mount that shares or nests a read-only mount's
    directory, since a file written through one view would load through the
    other — by directory identity, so a bind mount or firmlink of one
    directory is refused as that directory. The `writable` flag on a mount
    is therefore also its code-root policy; there is no separate mark.
  - A `path` or `registry` the requester names admits raw wasm alone
    (`wasm_only`): a pre-compiled artifact under it is refused however it
    hashes. The `endpoint` a registry load names serves a namespace the
    deployment's `registries` routes nowhere; a package the deployment
    routes is refused rather than fetched elsewhere.
  - A `[[guest]]` entry marked `on_demand = true` (the macro's
    `on_demand: true`) is admitted at its first `declared` load rather than
    at boot, from the source the entry declares: `source.path`, embedded
    bytes, or the new `source.package = "ns:name@1.0.0"` (the macro's
    `package:`), an exact reference fetched through `[registries]`. A
    package source is always on demand and must carry a `name`; an
    on-demand guest takes no `routes` and is never the `command` guest,
    since trigger routing is built at boot — each is refused when the
    manifest validates or, in the macro, at compile time.
    `GuestEntry::on_demand()` and `SourceSpec::package(..)` build them
    programmatically; the test host's `Deployment::on_demand(name, source)`
    and `Deployment::entry(entry)` declare them.
  - A declared entry's digest pin lives on the entry: `[[guest]]
    digest = "sha256:<hex>"` (the macro's `digest:`, `GuestEntry::digest`)
    is checked wherever the entry's bytes become a guest, at boot or on
    demand, before wasmtime sees them; a boot guest whose bytes miss the pin
    fails startup, an on-demand one refuses the load. A `path` or
    `registry` load carries its pin on the call, as before.
  - `omnia::Digest` is the typed `sha256:<hex>` value host-side — `Copy`,
    `Digest::of(bytes)`, `FromStr`/`Display` in the canonical lowercase
    spelling, serde as that string — and replaces
    `sha256_digest(bytes) -> String` (a `ContentStore` that verified with it
    compares `Digest::of(bytes).to_string()` instead). The registry records
    the digest of every guest it admitted (`LoadedGuest::digest`,
    `Guest::digest`), and the loader's `Plugin::digest` reports it on an
    attested name.
  - `omnia-plugin`'s surface is `Plugins` (installed by assembly over the
    on-demand `Source` table, the runtime's mounts, and the registry
    source), `PluginLoader::load(from: Location, pin: Option<Digest>)` on
    `Runtime`, `Plugin`, `Location::{Declared, Path, Registry}`, and the
    `RegistrySource` seam, whose `acquire(package, endpoint)` takes the
    registry the load names; `PathMounts` and `PathSource` are gone.
  - In the SDK, `Plugins::load(&Location, Option<&Digest>)` returns
    `Plugin { id, digest }`; `PluginRef` is gone, and `Location::name()` is
    the name a load registers under. The test host's `ScriptedLoader`
    declares the names a `Declared` load may resolve (`declare`), keys its
    `digest` and `refuse` scripts by that name, holds a call's digest
    against the resolved one, and records `loads()` as
    `(Location, Option<Digest>)` in call order.
  - Validating at load that a guest exports what its caller will import —
    refused typed at `load` rather than trapping at the first dispatch — is
    filed, not shipped, with its two homes: an `expects: list<string>` on
    the WIT `load` call the loader checks against the component's exports,
    or a check the requester makes against the returned handle once the
    loader exposes them.

### Removed

- `omnia_wasi_otel::set_filter`. A guest's tracing filter is the `RUST_LOG`
  its WASI environment carries, which the runtime sets from its verbosity
  flags (above); nothing reloads it at run time.

---

Release notes for previous releases can be found on the respective release branches of the repository.

<!-- ARCHIVE_START -->
* [0.36.x](https://github.com/augentic/omnia/blob/release-0.36.0/RELEASES.md)
* [0.35.x](https://github.com/augentic/omnia/blob/release-0.35.0/RELEASES.md)
* [0.34.x](https://github.com/augentic/omnia/blob/release-0.34.0/RELEASES.md)
* [0.33.x](https://github.com/augentic/omnia/blob/release-0.33.0/RELEASES.md)
* [0.32.x](https://github.com/augentic/omnia/blob/release-0.32.0/RELEASES.md)
* [0.31.x](https://github.com/augentic/omnia/blob/release-0.31.0/RELEASES.md)
* [0.30.x](https://github.com/augentic/omnia/blob/release-0.30.0/RELEASES.md)
* [0.29.x](https://github.com/augentic/omnia/blob/release-0.29.0/RELEASES.md)
* [0.28.x](https://github.com/augentic/omnia/blob/release-0.28.0/RELEASES.md)
* [0.27.x](https://github.com/augentic/omnia/blob/release-0.27.0/RELEASES.md)
* [0.25.x](https://github.com/augentic/omnia/blob/release-0.25.0/RELEASES.md)
* [0.23.x](https://github.com/augentic/omnia/blob/release-0.23.0/RELEASES.md)
* [0.22.x](https://github.com/augentic/omnia/blob/release-0.22.0/RELEASES.md)
* [0.21.x](https://github.com/augentic/omnia/blob/release-0.21.0/RELEASES.md)
* [0.20.x](https://github.com/augentic/omnia/blob/release-0.20.0/RELEASES.md)
* [0.19.x](https://github.com/augentic/omnia/blob/release-0.19.0/RELEASES.md)
* [0.18.x](https://github.com/augentic/omnia/blob/release-0.18.0/RELEASES.md)
* [0.17.x](https://github.com/augentic/omnia/blob/release-0.17.0/RELEASES.md)
* [0.16.x](https://github.com/augentic/omnia/blob/release-0.16.0/RELEASES.md)
* [0.15.x](https://github.com/augentic/omnia/blob/release-0.15.0/RELEASES.md)
