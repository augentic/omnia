## 0.37.0

Unreleased

### Added

- One tracing filter for the whole process, its level selected by verbosity
  flags and composed with `RUST_LOG`. A run's bare level is the flag, else
  the process `RUST_LOG`'s, else the mode's default — `info` for a command,
  `warn` for a server (`Mode::level`) — and the process `RUST_LOG`'s
  targeted directives (`tower=off`, `my_sdk=debug`) apply on top whichever
  decided the level, so `RUST_LOG=my_sdk=debug bin -v` runs at
  `debug,my_sdk=debug` and `RUST_LOG=info bin -v` at `debug`; a token that
  is neither a level nor a directive is reported and dropped, as `EnvFilter`
  drops it. The filter governs the host console and every guest alike: each
  guest's WASI environment carries it as `RUST_LOG`, and the process
  environment is never written. `omnia::telemetry::directives(level,
  fallback, rust_log)` is the composition, pure over its inputs. Each
  `-v`/`--verbose` steps the level one rung up the scale `off`, `error`,
  `warn`, `info`, `debug`, `trace` from the mode's default and each
  `-q`/`--quiet` one rung down, clamped at the ends, so a command reads `-q`
  warn, `-qq` error, `-v` debug, `-vv` trace, and a server `-q` error, `-qq`
  off, `-v` info, `-vv` debug, `-vvv` trace. On the `run` grammar the flags
  are global to the command (`bin -v run …`, `bin run -v …`; `-v` beside
  `-q` is a usage error); on the direct-command path the host reads them out
  of argv before `--` and forwards argv verbatim, so a command guest
  declares the same flags by flattening `omnia_sdk::api::command::Verbosity`
  into its grammar, which lists them in help and completions, accepts them,
  and refuses the pair with its own usage error — the guest never acts on
  the values. Embedders select the level in code with
  `DeploymentBuilder::level(LevelFilter)` (`omnia` re-exports
  `LevelFilter`), `Telemetry::filter(directives)` sets the console's filter
  outright (the run's composed directives, in omnia's own deployment),
  `Telemetry::fallback(level)` sets its fallback for an unset `RUST_LOG`
  (`WARN` when not called, as before), and the test host's
  `Deployment::level` scripts a guest's bare level without touching the
  suite's environment. `RuntimeParts` carries the composed `rust_log:
  String` where it carried `level` and `fallback`. A bare command run's
  host console now opens at `info`, where it opened at `warn`.
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
- `omnia-wasi-vcs`: the `omnia:vcs` capability, version control over a
  guest's lent mounts. The WIT package carries three interfaces — `store`
  (`resolve`, `head`, `commit`, `merge` under a policy of glob rules),
  `workspace` (`init`, `add`, `remove`, `pending`), and `transport`
  (`clone`, `fetch`, `label`, `push`) — every function async, each naming a
  repository or working copy as a `location`: a borrowed mount-root
  descriptor plus a relative subpath. The host (`WasiVcs`, a backend behind
  `WasiVcsCtx`) resolves the location beneath the authorized mount before
  the backend runs, refusing a descriptor that is not a mount root, a
  subpath that leaves the mount or starts at `/`, and a mutation beneath a
  read-only mount, and lays down a subpath nothing holds yet for the
  operation that creates it. The backend receives each location as a
  `Place`: an open directory handle beneath the mount, held for as long as
  the backend runs, so a guest rearranging its tree meanwhile cannot
  redirect the operation. A typed `error` the backend returns crosses as its
  variant; any other failure lowers to `other` with its detail. The crate
  ships no default backend — a repository is a tree on disk — so a
  deployment that links `WasiVcs` names one (`omnia-git` in
  `omnia-backends`), and the test host's `Backends` holds `NoVcs` in the
  slot until `Backends::vcs` sets a `WasiVcsCtx`. `omnia_sdk::vcs` is the
  guest capability: the `Vcs` trait over deployment-local paths (`"."` the
  project mount, `"./sub"` or `"/mount/sub"` beneath one), resolved against
  the preopens by the lend rule `Model::complete` already uses for its
  workspace, with mirrored `Rule`, `Strategy`, `Merged`, `Change`,
  `ChangeKind`, `CloneOptions`, and a `vcs::Error` carrying its wire
  `code()` and a default taxonomy onto `omnia_sdk::Error`; the transport
  `clone` is `Vcs::clone_repo`, so a `Clone` provider keeps its `clone`.

### Changed

- `Format::candidate` reads the largest JSON value a reply wraps, where it
  read the last — a citation or a repeated fragment after the answer is no
  longer taken for it — and passes over a bracketed block that does not
  parse whole, where it read the values inside it: a document cut short is
  handed back as written, so the guest's check sees the reply rather than
  a well-formed member of it. A fence body is one such block, so a literal
  inside code is never taken for the answer beside it; a fence is delimited
  only by a "```" that begins a line, so one inside a string or a sentence
  is content.
- `omnia_wasi_model::Usage` is the WIT `usage` record itself, as `Reply`
  and `Request` already were, rather than a mirror of it; the mirror's
  `From` conversion goes with it. The host-only types drop the derives
  nothing exercised: `Answer`, `Transcript`, `ToolTurn`, `Sections` and
  `Example` lose `PartialEq`/`Eq`, `Transcript`, `ToolTurn` and `DirEntry`
  lose `Serialize`/`Deserialize` (`DirEntry` keeps `Serialize`, which
  backends use to put a listing to the model), and `Example` loses
  `Default`.
- A run with no collector exports nothing. A signal's OTLP exporter attaches
  only when an endpoint is configured for it — OpenTelemetry's
  `OTEL_EXPORTER_OTLP_ENDPOINT` or the signal's own
  `OTEL_EXPORTER_OTLP_{TRACES,METRICS}_ENDPOINT`, an empty value counting
  as unset — where it used to fall back to `localhost:4317` and, without a
  collector there, spend every run retrying the connection and every exit
  waiting on the flush. The
  providers still publish (the resource and the tracer guest telemetry
  grafts onto are unchanged); an unexported signal is dropped, and the
  choice is reported once at `debug`. `tower` joins the always-muted
  targets, so the retries a configured-but-unreachable collector does
  produce stay off the console too. Per-operation narration — every call
  on the in-memory `keyvalue`, `blobstore`, `vault`, `messaging`, and `sql`
  hosts, the `wasi-http` guest's request and response dumps, and
  `wasi-keyvalue`'s `Cache` reading a value it did not write — moves from
  `debug` to `trace`, so `-v` shows a run's decisions and `-vv` its every
  step.
- The host console shows the runtime's own crates — `omnia`, `omnia_core`,
  `omnia_link`, `omnia_plugin` (`telemetry::RUNTIME`) — as a server run
  shows them, whatever the mode: from `warn`, stepping with the verbosity
  flags (`-v` info, `-vv` debug, `-vvv` trace), or at the process
  `RUST_LOG`'s bare level as written. A bare command run's console carries
  its guests' progress and the runtime's warnings alone; `-v` adds the
  runtime's `info` beside the guests' `debug`. A `RUST_LOG` directive
  naming one of those crates, or `omnia` as a prefix of all four, stands.
  The refinement is the console layer's own filter and nothing else's:
  spans, the exporters, and every guest's `RUST_LOG` follow the run's
  filter, so the `cli-run` span guest telemetry grafts onto is live at a
  bare command run whether or not the console prints it — host lines
  inside the run carry the `cli-run:` prefix from `-v`, where the console
  first sees the span. `Telemetry::runtime_level(level)` sets the
  refinement and `telemetry::bare(level, fallback, rust_log)` reads the
  level `directives` leads with; embedders select the level with
  `DeploymentBuilder::runtime_level`, which the generated `main` sets from
  the flags. With the refinement in place, a guest's load (`guest loaded`,
  with its digest), each `wasi:cli/run` exit, and the `omnia ready` line
  are at `info` in either mode, where the command-mode lines were at
  `debug`, and a command deployment with no `wasi:cli/run` exporter warns
  that it is inert, where it said so at `info`.
- Console colour only when stderr is a terminal. A redirected or captured
  stderr (`2>run.log`, a container log driver, journald) gets plain text
  where it used to get ANSI escapes; on a terminal `NO_COLOR` still
  disables it.
- The runtime opens an `info` span around every `wasi:cli/run` drive
  (`cli-run`) and every trigger request (`http-request`,
  `messaging-handle`, `websocket-handle`, up from `debug`). Guest spans
  graft onto the host span live when they export and are dropped without
  one, so a command run's guest spans now reach a configured collector at
  the default level, where no span was live around the drive at any level;
  a server's request spans are live from `-v` (`RUST_LOG=info`), its
  default `warn` still dropping them. Host console lines emitted inside a
  live span carry its name (`cli-run:`), as `fmt` renders span context.
- A guest not compiled into the runtime loads at its first use. Bytes that
  were in the process before any guest ran — the macro's embedded `path:`,
  the component `run <component>` names, a programmatic
  `GuestEntry::new(name, bytes)` — load at boot; every other `[[guest]]` —
  a manifest file's `source.path` or `source.package`, the macro's
  `package:`, a programmatic path entry — loads the first time a route, the
  command drive, a link call, a host dispatch, or an
  `omnia:plugins/loader.load(declared(..))` names it. Nothing marks the
  difference: the source kind is the rule, so there is no `on_demand` key
  anywhere (TOML, macro, `GuestEntry`), and a manifest carrying one is
  refused as an unknown key. Boot still validates everything the manifest
  states; what a compiled-in guest would fail at boot — a missing file, a
  pin miss, a refused artifact, a routed target that does not export the
  trigger's handler — a first-use guest fails at the use that named it
  (`500` and an `error` log on HTTP, a dropped event on messaging and
  websocket, the error chain on the command drive and a link call, a typed
  `error` on `load`). A first-use guest is never a trigger's catch-all: the
  no-routes fallback is drawn from the guests loaded at boot, so a manifest
  file's command guest carries `command = true` (the `model` and
  `guest-link` example manifests do now). Concurrent first uses may load
  twice; the second admission loses and returns the winner.
  - `Runtime::guest(&id)` is the one seam: a registered guest as it stands,
    a declared one read (or fetched, for a package), verified, and admitted
    as a late guest, typed by `GuestError { Unregistered, Unavailable,
    Refused, Internal }` (`Unregistered` displays as the former "guest `..`
    is not registered"). `dispatch`, the command driver, the trigger hosts,
    the link relay (through the new `Dispatcher::ensure(&id)`), and the
    loader's `declared` arm all resolve through it.
  - Admission is one body. `Runtime::admit_bytes(id, bytes, policy)`
    verifies bytes under a `Policy` — `Declared(&Source)`, the entry's pin
    and the format its source kind admits; `CallerNamed { pin }`, the
    call's pin and raw wasm alone — seats them, and answers the guest
    standing under the id afterwards (a declared entry attests a racing
    first use; caller-named bytes attest on the same digest alone, and are
    refused under a name active with other bytes). `Runtime::acquire(spec,
    endpoint)` produces a source's bytes — a path read, a package fetched
    through `RuntimeParts.packages`, embedded bytes as they are. `guest`,
    `register`, and the loader's `path` and `registry` arms all pass through
    `admit_bytes`, so the rule that a name the deployment declares is bound
    by its entry alone (`Runtime::admits(id, &policy)`, the one home of the
    rule) holds for all of them: `Runtime::register` and `Runtime::admit`
    under a declared name are refused where they used to seat the
    embedder's bytes for the next declared load to attest, `register` of
    the bytes already active under an id attests instead of failing, and
    `admit` returns `GuestError` (`AdmitError` is gone). The loader's
    `Admission` seam is `admits`, `guest`, `acquire`, and `admit_bytes`
    over `WeakRuntime`, and the loader refuses a path or package deriving a
    declared name before any read or fetch, through the runtime's rule.
  - `Registry::assemble(RegistryParts { engine, linker, options, loaded,
    declared, routes, seam, allow_empty })` takes the declared `Source`s
    beside the loaded guests (`Registry::declared(&id)`, `is_declared`); a
    name in both, or twice, is the duplicate error, the empty check is "no
    loaded and no declared guest", and a route may target a declared guest.
    `TriggerRouter<R>` (`Runtime::http_trigger_router` follows) caches no
    handler indices: `resolve(key)` and `catch_all()` yield the `GuestId`,
    and each trigger host probes the resolved guest's exports per event.
  - `RegistrySource` and its `AcquireError { Refused, Unavailable }` live in
    `omnia-core` (re-exported by `omnia` and `omnia-plugin`) as the acquirer
    `RuntimeParts.packages: Option<Arc<dyn RegistrySource>>` fetches every
    package through — a `source.package` guest at first use and a package a
    load names alike; `Deployment::assemble` sets it to the loader's
    `RegistryClient` under the `loader` feature, and a manifest declaring a
    package guest without the feature is refused at startup beside one
    declaring `registries`. `Plugins::install(runtime)` takes no guest
    table and no registry: the loader's grant is the runtime's mounts, and
    `From<AcquireError> for LoadError` is gone with the loader's own fetch.
  - `Manifest::sources()` is the one list (`boot_sources()` and
    `on_demand_sources()` are gone; `Deployment::build` partitions it on
    `SourceSpec::Bytes`), `Manifest::from_wasm(path) -> Result<Self>` reads
    the component now so `run <component>` loads at boot in either format,
    and `omnia_test::host::Deployment::guest(name, path)` reads the path
    into bytes for the same reason; `Deployment::on_demand` is gone,
    `Deployment::entry(GuestEntry::new(name, path))` declares a first-use
    guest.
- One loader for either artifact format, everywhere a guest loads. The
  bytes a declared guest resolves to are deserialized when they are `omnia
  compile` output and compiled when they are raw wasm, told apart the way
  wasmtime tells them apart (`Engine::detect_precompiled`); a
  settings-mismatched pre-compiled artifact still fails with the
  compile-settings hint. The raw-wasm/pre-compiled trust split is gone with
  its API: `DeploymentBuilder::build_trusted` (`build` is the one build, and
  the generated `run` / `run_with` load a pre-compiled guest as `main`
  does), `GuestArtifact`, `is_precompiled`, and `ELF_MAGIC`. Which format a
  source may load follows from the source kind alone, with no key
  (`docs/security-model.md`): embedded bytes and `run <component>`, in the
  process before any guest ran, load either; a `source.path`, read at first
  use while guests run, loads `omnia compile` output only when its entry
  pins the `digest` ("is pre-compiled and is read from unpinned .. while
  guests run: pin its `digest`"); a `source.package`, a caller-named `path`
  or `registry` load, and `Runtime::register(id, bytes)` admit raw wasm
  alone ("a package admits raw wasm alone"; "`Verified::wasm` admits raw
  wasm alone"), because a registry, a requester, or an embedder's untrusted
  bytes are not the deployment's build — an artifact the embedder's own
  build produced goes through `Runtime::admit` wrapped in the `unsafe`
  `Verified::trusted`. The `omnia:plugins/loader` `refused` variant names
  each of those.
  - Native code is admitted through one token. `omnia::Verified` is
    component bytes with their digest, obtained from exactly three places:
    `Source::verified`, once the pin and the format rule have passed;
    `Verified::wasm`, which admits raw wasm alone; and the `unsafe`
    `Verified::trusted`, the embedder's word for an artifact of its own.
    `Runtime::admit(id, verified)` takes it (where it took bytes and a
    digest), and `register` and the loader's caller-named loads go through
    `Runtime::admit_bytes`, which verifies under its `Policy` and hashes
    once.
  - The digest is never optional. Every path a guest's bytes take runs
    through one body, `Runtime::admit_bytes`, and every registration
    records the digest of the bytes it was loaded from: `Guest::digest` and
    `LoadedGuest::digest` are a `Digest`, `Plugin::digest` is a `Digest` on
    the host and a `&Digest` in the SDK (the WIT record's `digest` is a
    `string`), and a load of an active guest always attests it.
    `Guest::with_digest` and the test loader's `ScriptedLoader::unhashed`
    are gone with the `None` they scripted. `Digest::checked(bytes, pin,
    subject)` is the one pin check ("resolved to .., not its declared digest
    ..") `Source::verified` and the caller-named policy share.
  - One `Source` for every declared guest. `omnia::Source` (in `omnia-core`,
    beside the `SourceSpec` that moved there under the same
    `omnia::SourceSpec` path) is a `[[guest]]` entry resolved for loading —
    identity, `SourceSpec`, digest pin — with `read`, `verified`, and
    `load`, so the pin check and the format rule are one body at boot and
    at first use; omnia-plugin's `Origin` and `OnDemand` are gone. A
    caller-named `path` or `registry` load builds no `Source`: it runs the
    shared pin check, then `Verified::wasm`.
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
  `omnia::telemetry::flush`, `omnia::telemetry::resource`, and the
  `OTEL_EXPORTER_OTLP_*` endpoints are never compiled out, and a
  `default-features = false`
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
    command?, digest? }, .. ]`.
    Each entry's `path:` is a string literal or a macro expanding to one
    (`concat!(env!(..), ..)`, a `build.rs`-emitted `env!("GUEST_WASM")`) that
    the macro reads with `include_bytes!` — the component is compiled into
    the binary and loads at boot, never read from disk; a component read at
    its first use is a `manifest:` file's `[[guest]]`. A computed `path:` is
    a compile error. A `package:` entry needs nothing but its reference —
    it takes `routes:`, `command:`, and `digest:` like any other, and
    `name:` is optional — and any other key, the retired `on_demand:` and
    `wasm_only:` included, is a compile error naming the six.
  - A guest is named by the path's file stem, or by the package reference
    without its version, unless `name:` says otherwise.
  - `registries:` is a root key beside `guests:` and `mounts:`, the routing
    of the deployment's `package:` guests.
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
    `echo`) or the `source.package` reference without its version does
    (`acme:echo@1.2.0` is `acme:echo`; `Manifest::load` derives both). Two
    versions of one package collide as a duplicate name unless one is
    given a `name`. The retired `id` key is an unknown key.
  - `GuestEntry::new(name, source)`, `GuestEntry::embedded(path, bytes)`
    (named by the path's stem) and `GuestEntry::package(reference)` (named
    by the reference without its version) — what the macro lowers to —
    `GuestId::from_path`, `GuestId::from_package`; the test host's
    `Deployment` takes `guest(name, ..)` and `command(name)`.
- The `link:` and `plugin:` blocks, `omnia::Location`, `LinkConfig`, and
  `PluginConfig` are gone; `config:` is `manifest:`.
  - The TOML manifest follows: `[registries] path = "wasm-pkg.toml"` replaces
    `[[plugin.location]]`, and `Manifest::load` replaces
    `Manifest::from_config`. `omnia_test::host::Deployment::path_root` is
    gone with the location list, and the test deployment's `locations`
    builder is `registries`.
  - The guest loader is assembly's, not the macro's: `Deployment::assemble`
    links `WasiPlugins` beside WASI whenever omnia is built with the
    feature, hands the runtime its `RegistryClient` as the package source
    (above), and installs the loader's grant through
    `Plugins::install(runtime)`; `Wiring::extend` and
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
    declared name, the path's file stem, or the package reference without
    its version (`registry("acme:tool@1.2.3")` registers `acme:tool`, so a
    deployment holds one guest per package and another version of an
    active one is refused as active under other bytes; the SDK's
    `Location::name` strips the version the same way). A `declared` name
    outside the deployment's guest list is refused typed (where the same
    name dispatched blind would trap), one not yet loaded is loaded through
    `Runtime::guest` — the same first-use seam a route or a link call uses
    — one already active is attested without fetching anything, and it
    takes no digest of its own — the entry carries the pin. The
    `already-active` variant is gone: a location whose name is active under
    other bytes is `refused`, and the same bytes attest. A `path` or
    `registry` whose derived name the deployment declares — any version of
    a declared package included — is `refused` before anything is read,
    whether or not that guest has loaded: a declared name is bound by its
    entry alone.
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
    (`Verified::wasm`): a pre-compiled artifact under it is refused however
    it hashes. The `endpoint` a registry load names serves a namespace the
    deployment's `registries` routes nowhere; a package the deployment
    routes is refused rather than fetched elsewhere.
  - `source.package = "ns:name@1.0.0"` (the macro's `package:`,
    `SourceSpec::package(..)`, `GuestEntry::package(..)`) declares a guest
    as an exact package reference acquired through `[plugins]` at its
    first use — a route, the command drive, a link call, a host dispatch,
    or a `declared` load — and named by the reference without its version
    unless the entry names it. It takes `routes` and `command` like any
    other entry, and admits raw wasm alone.
  - A declared entry's digest pin lives on the entry: `[[guest]]
    digest = "sha256:<hex>"` (the macro's `digest:`, `GuestEntry::digest`)
    is checked wherever the entry's bytes become a guest, at boot or at
    first use, before wasmtime sees them; a guest loaded at boot whose
    bytes miss the pin fails startup, a first-use one fails the use that
    named it. On a `source.path` the pin is also what admits `omnia
    compile` output. A `path` or `registry` load carries its pin on the
    call, as before.
  - `omnia::Digest` is the typed `sha256:<hex>` value host-side — `Copy`,
    `Digest::of(bytes)`, `FromStr`/`Display` in the canonical lowercase
    spelling, serde as that string — and replaces
    `sha256_digest(bytes) -> String`. The registry records
    the digest of every guest it admitted (`LoadedGuest::digest`,
    `Guest::digest`), and the loader's `Plugin::digest` reports it on an
    attested name.
  - `omnia-plugin`'s surface is `Plugins` (installed by assembly over the
    runtime's mounts and the registry source; the declared guests are the
    registry's), `PluginLoader::load(from: Location, pin: Option<Digest>)`
    on `Runtime`, `Plugin`, `Location::{Declared, Path, Registry}`, and the
    `RegistrySource` seam re-exported from `omnia-core`, whose
    `acquire(package, endpoint)` takes the registry the load names and
    fails with core's `AcquireError` (`From<AcquireError> for LoadError`,
    `From<GuestError> for LoadError`); `PathMounts` and `PathSource` are
    gone.
  - In the SDK, `Plugins::load(&Location, Option<&Digest>)` returns
    `Plugin { id, digest }`; `PluginRef` is gone, and `Location::name()` is
    the name a load registers under. The test host's `ScriptedLoader`
    declares the names a `Declared` load may resolve (`declare`), keys its
    `digest` and `refuse` scripts by that name, holds a call's digest
    against the resolved one, and records `loads()` as
    `(Location, Option<Digest>)` in call order.
  - A loaded handle says what the guest exports. The WIT `plugin` record
    gains `exports: list<string>` — each exported interface by its full id
    (`acme:tool/run@1.0.0`) and each bare export by name — so a requester
    checks that a guest exports what it will import before the first
    dispatch, refusing it typed instead of trapping; `omnia_sdk::plugins::
    Plugin::exports()` reads it, `Plugin::new(id, digest, exports)` builds
    one, and the host-side `omnia_plugin::Plugin` carries the same list
    (`Plugin::of(&guest)`), computed once at admission from the component
    type (`Guest::exports`). The test host's `ScriptedLoader::exporting(name,
    exports)` scripts the list a name resolves with. A loader-side
    `expects` check stays filed.
- Every package comes through the deployment's package store. A
  `source.package` guest at its first use and a package a `registry` load
  names are acquired by the same `RegistryClient`, store first: the
  `PackageStore` the deployment names answers before any registry, and a
  release it lacks is fetched by the `registries` routing, verified against
  the registry's digest, and written to the store once. The store keeps
  the deployment's word alone: a release a load fetched from the
  `endpoint` it named, for a namespace the routing leaves unrouted, is
  served to that load and never stored, so nothing a loading guest names
  becomes what a later acquisition of the reference — a declared
  `source.package` guest's first use included — is served. The store is a
  flat directory of one file per release, named by the reference under
  the `_` spelling — `acme:tool@1.2.3` is `acme_tool@1.2.3.wasm` — and
  only that spelling is read, so `tool.wasm` beside it, an unversioned
  name, and a tool's temporary files are not releases. A stored release
  is served whoever wrote it, whatever `endpoint` the load names, and
  with no network: `wkg get acme:tool@1.2.3 -o <store>/` or a plain `cp`
  stage one by hand, a build that copies its own component there runs it
  with nothing fetched, and the one INFO line per acquisition says which
  answered. `put` never replaces a file already there, so removing the
  file is the refresh; a pre-compiled release a registry serves is refused
  before the write, so the store never holds one; and the store may not
  lie beneath a writable mount — present, or a directory a guest could
  create through the mount, judged by the `(device, inode)` ancestry of
  its nearest existing ancestor (`MountRegistry::beneath_writable`,
  `MountRegistry::ancestry`) — since a guest could then write what the
  deployment loads; `Deployment::build` refuses it naming the mount.
  - The manifest names both under one table. `[plugins] store = "~/.app/
    store"` and `[plugins] registries.path = "wasm-pkg.toml"`
    (`Manifest.plugins: Option<PluginsConfig { store, registries }>`,
    the fluent `Manifest::store(path)` and `Manifest::registries(config)`,
    `Manifest::store_root()`, `Manifest::registry_config()`) replace the
    top-level `[registries]` table, which is an unknown key now; either
    key may stand alone, and a manifest naming either without the `loader`
    feature is refused at startup as a package guest is. In the `runtime!`
    macro the block is `plugins: { store: "~/.app/store", registries:
    include_str!("wasm-pkg.toml") }`; a top-level `registries:` is a
    compile error pointing at the block, as are an empty block and an
    unknown key in it. A leading `~/` on the store, or on a mount's path,
    is the operator's home, expanded against `$HOME` when the deployment
    builds (`Manifest::expand_home`; unset, the build fails) — the one
    place a manifest reads the environment.
  - `RegistryClient<S: PackageStore>` is built over its store —
    `RegistryClient::new(config, store)`, `RegistryClient::from_toml(
    Option<&str>, store)` (`None` is an empty routing) — and
    `Deployment::assemble` installs one over an `FsStore::open(root)` when
    the manifest names a store (a missing root is an empty store, created
    at the first write) and over `NoStore` otherwise, where every fetch is
    served and nothing kept. `PackageStore { get, put, describe }` is the
    trait a store of your own implements, keyed by `Reference` (the parsed
    `ns:name@version`, `Display` in that spelling, `file_name()` in the
    store's), and `describe` supplies the refusal's last clause: a package
    the routing leaves nowhere is refused before any fetch, naming the
    reference, its namespace, and where the store looked ("the store
    `<root>` holds no `<file>`"). A `put` that fails warns and the release
    is served from the fetched bytes; a `get` that fails is the load's
    error.
  - A guest's stdout and stderr are the embedder's to capture.
    `StoreConfig::stdio(Stdio)` (`omnia::Stdio::new(stdout, stderr)`, two
    shared `StdoutStream`s) hands every store a sink pair where it
    inherited the process's; `DeploymentBuilder::stdio(..)` sets it for a
    deployment, and the test host's `Deployment::captured()` wraps a
    deployment so its `run`, `run_host`, and `run_with` answer a `Run {
    status, stdout, stderr }` — the whole of what the guests wrote, over
    in-memory pipes — while `Deployment::store(root)` overlays the store
    root the way `mount` overlays a mount.

### Removed

- `OTEL_GRPC_URL`. The host's collector endpoint is OpenTelemetry's own
  `OTEL_EXPORTER_OTLP_ENDPOINT` (with `OTEL_EXPORTER_OTLP_{TRACES,METRICS}_ENDPOINT`
  per signal), which the exporter already resolved beneath the alias; the
  alias was read by the runtime alone and handed to `Telemetry::endpoint`,
  which stays for an embedder setting the endpoint in code. `omnia-opentelemetry`
  in `omnia-backends` reads the same variable for guest telemetry, so one
  setting now names the collector for both.
- `omnia_wasi_otel::set_filter`. A guest's tracing filter is the `RUST_LOG`
  its WASI environment carries, which the runtime composes from its
  verbosity flags and the process `RUST_LOG` (above) — the same
  directives-then-`RUST_LOG` layering `set_filter` did for one guest, now
  decided once for the whole process; nothing reloads it at run time.
- `RegistryClient::cached`, `ContentStore`, and `ReleaseStore`. The two
  caches keyed by endpoint, reference, and digest are one `PackageStore`
  keyed by `Reference` (above), which every `RegistryClient` carries; a
  store implemented against the old traits implements `get`, `put`, and
  `describe` instead. `omnia-backends` drops its filesystem store for the
  built-in `FsStore` and re-points its Azure Blob store at the trait.
- The per-start refresh of a `package:` guest. A release the store holds
  is never fetched again while its file stands, whatever registry the
  deployment routes its namespace to or a load names: a deployment that
  expected each start to pick up a re-pushed tag removes the stored file
  to refresh, and one that wants a new release names a new version. A
  `digest` pin on the entry or the load holds the stored bytes as it held
  the fetched ones.

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
