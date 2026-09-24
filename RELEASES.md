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
  - `guests:` is a list, `guests: [ { path, name?, routes?, command? }, .. ]`.
    Each entry's `path:` is a string literal or a macro expanding to one
    (`concat!(env!(..), ..)`, a `build.rs`-emitted `env!("GUEST_WASM")`) that
    the macro reads with `include_bytes!` — the component is compiled into
    the binary, never read from disk at start; a component read at start is
    a `manifest:` file's `[[guest]]`. A computed `path:` is a compile error.
  - A guest is named by the path's file stem unless `name:` says otherwise.
  - `registries:` is a root key beside `guests:` and `mounts:`, the default
    routing of a package load that names no registry.
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
    `echo`, `Manifest::load` derives it). An entry nothing names — an `oci`
    source without `name` — is refused at startup beside a duplicate name.
    The retired `id` key is an unknown key.
  - `GuestEntry::new(name, source)`, `GuestEntry::embedded(path, bytes)`
    (named by the path's stem; what the macro lowers to),
    `GuestId::from_path`; the test host's `Deployment` takes
    `guest(name, ..)` and `command(name)`.
- The `link:` and `plugin:` blocks, `omnia::Location`, `LinkConfig`, and
  `PluginConfig` are gone; `config:` is `manifest:`.
  - The TOML manifest follows: `[registries] path = "wasm-pkg.toml"` replaces
    `[[plugin.location]]`, and `Manifest::load` replaces
    `Manifest::from_config`.
  - Mounts double as the roots path loads resolve against (`./plugin.wasm`
    reads from the `.` mount), so there is no separate root list and
    `omnia_test::host::Deployment::path_root` is gone — overlay a `.` mount
    instead (mounts dedup by name, last wins, before any directory opens);
    the test deployment's `locations` builder is `registries`.
  - The guest loader is assembly's, not the macro's: `Deployment::assemble`
    links `WasiPlugins` beside WASI whenever omnia is built with the feature
    and installs the declared policy — mounts and `registries` — or the
    custom sources selected with the new
    `Deployment::loader(registry, path)`; `Wiring::extend` and
    `Deployment::plugin_locations` are removed, and the generated `Hooks`
    carry `link` and `serve` alone.
  - `RegistryClient::new` takes the wasm-pkg `Config` (there is no
    `with_config`) and is always installed, over an empty configuration when
    the deployment declares none. The configuration is routing, not an
    allow-list: it routes the loads that name no registry — a package it
    routes nowhere (no `default_registry`, no mapping for its namespace) is
    refused typed, naming the namespace, before any registry is dialled —
    and a load that names its registry fetches from it whatever the
    configuration says.
  - The `plugin` cargo feature is `loader`.
  - On the CLI, `--config`/`-c` is `--manifest`/`-m`, `OMNIA_CONFIG` is
    `OMNIA_MANIFEST`, and `--link` is gone; `RunSource::Config` is
    `RunSource::Manifest`.
- The `omnia:plugins/loader` contract names every kind of guest a caller can
  reach through one `load`.
  - `location` is `registry(registry-ref)`, `path(string)`, or
    `declared(string)`. A `registry-ref` carries the package and an optional
    `endpoint`, the registry the load fetches from — a load naming none takes
    the deployment's `registries` routing, and a package neither routes is
    refused before any registry is dialled. A path reads through the
    deployment's mounts as before. `declared(name)` answers with the
    deployment's own guest of that name, attesting the registration without
    fetching anything and refusing an undeclared name (where the same name
    dispatched blind would trap) or a pin (a declared guest is the
    deployment's, not the caller's, to pin).
  - `load(from, digest)` takes the pin beside the location, and
    `plugin.digest` is optional — a declared guest carries none.
  - A path load registers under its file stem (`./x.wasm` is the guest `x`;
    `GuestId::from_path`), so a caller dispatches to what it loaded by the
    name it loaded it as.
  - In the SDK, `Location` follows the variant, `Location::name` answers the
    registration name a location derives, `PluginRef` is a plain
    `{ location, digest }` struct, and `Plugin::digest` is
    `Option<&Digest>`; the test host's `ScriptedLoader` keys its digests and
    refusals by that name and `declare(name)` scripts the deployment's
    guests.
  - Validating at load that a guest exports what its caller will import —
    refused typed at `load` rather than trapping at the first dispatch — is
    filed, not shipped, with its two homes: an `expects: list<string>` on
    the WIT `load` call the loader checks against the component's exports
    for all three kinds, or a check the requester makes against the returned
    handle once the loader exposes them.

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
