# Troubleshooting

Common failures and their fixes, grouped by when they bite. If your problem isn't here, run with `RUST_LOG=debug` — the runtime logs its decisions (manifest resolution, mount layering, backend connection, routing) at `debug`.

## Building

### `can't find the wasm file` / the guest built with the wrong name

Guest binaries use **underscores**, even when the cargo target uses hyphens: `cargo build --example http-wasm` produces `http_wasm.wasm`, not `http-wasm.wasm`. Check `target/wasm32-wasip3/debug/examples/` for the actual name.

### Building the whole workspace for `wasm32-wasip3` fails

Expected. Native host crates (wasmtime, tokio, backends) don't compile for the wasm target. Build guest targets explicitly:

```bash
cargo build --example <name>-wasm --target wasm32-wasip3
```

never `cargo build --workspace --target wasm32-wasip3`.

### `error[E0463]: can't find crate for 'wasip3'` on a native build

The `wasip3` crate exists only for the wasm target. Guest modules must be gated:

```rust
#![cfg(target_arch = "wasm32")]
```

and shared crates put wasm-only dependencies under `[target.'cfg(target_arch = "wasm32")'.dependencies]`.

### `cargo fmt` produces unexpected diffs or errors

Formatting uses nightly rustfmt: `cargo +nightly fmt --all`. The stable formatter doesn't understand the workspace's `rustfmt.toml` options.

## Starting the host

### The host prints nothing and appears hung

It's probably running fine — servers are quiet by default. Run with `-v` and look for the `omnia ready` line. Without it, the only output is Cargo's `Running ...`.

### `no guest specified: pass a <wasm> path, or --manifest <omnia.toml>`

The `run` subcommand needs either a positional `.wasm`/`.bin` path or a manifest via `--manifest`/`OMNIA_MANIFEST`. Also check argument order: flags for the *host* go before `--`, guest argv after it.

The embedder-path variant — `no deployment manifest supplied and OMNIA_MANIFEST is unset` — means a `DeploymentBuilder` was built without `.manifest(...)` and no `OMNIA_MANIFEST` fallback was available.

### `no guest ... is declared by this deployment`

A guest called `omnia:plugins/loader.load` with a `declared` name the manifest does not declare. A `declared` load names only `[[guest]]` entries (the macro's `guests:`); add one for the name — a manifest file's entry loads at that first `load` — or, for a component the deployment does not declare, have the guest name its `path` beneath a read-only mount or its `release` instead. A runtime built without omnia's `loader` feature has no loader at all: a guest importing it fails at instantiation, and a manifest declaring `plugins` or a `source.package` guest is refused at startup until the feature is enabled.

### `no registry routes ...`

A `source.package` guest (the macro's `package:`) was first used, or a guest named a `release` of its own; the deployment's store holds no release under that reference, and the `registries` configuration (`plugins: { registries: include_str!("wasm-pkg.toml") }` in the macro, `[plugins] registries.path` in a manifest) is absent or names neither a `default_registry` nor the package's namespace. The refusal names the store file that would have served the release: fetch it there (`wkg get <reference> -o <store>/`) or copy a component under that name, route the namespace, pin the package to its registry under `[package_registry_overrides]`, or — for a release a guest names — have the load name its `registry`, which serves a namespace the configuration routes nowhere (and is never written to the store, so every such load fetches).

### `... is routed to ... by the deployment's registries; it cannot be fetched from ...`

A guest named a `release` with a `registry` other than the one the deployment's `registries` routes its namespace to. The deployment's routing outranks the load's: drop the registry, or route the namespace to it in the configuration.

### `path ... is beneath the writable mount ...`

A guest named a component `path` beneath a mount the deployment marks `writable`. A component loads from a read-only mount alone — what a guest can write, it cannot run. Mount the code directory read-only and keep the guest's state under a mount of its own; the two may not share or nest directories (`writable mount ... shares its directory with the read-only mount ...` at startup, judged by directory identity, so a bind mount or firmlink of the code directory counts as it).

### `... is a guest this deployment declares, bound by its entry alone`

A guest named a `path` or `release` whose derived name — the path's file stem, the reference without its version — is a `[[guest]]` the deployment declares, whether or not that guest has loaded yet; or an embedder called `Runtime::register` or `Runtime::admit` under such a name. A declared name is bound by its entry alone, on every admission path, so nothing a caller names can seat other bytes under it: load the guest as `declared(name)`, or rename the file or the entry so the two no longer collide.

### `... resolved to sha256:..., not its declared digest ...`

The guest's bytes do not hash to the `digest` its `[[guest]]` entry pins. For a guest loaded at boot (embedded bytes, `run <component>`) this fails startup; for a first-use guest it fails the use that named it — the request, the run, the link call, or the `load`. Either the artifact changed under the deployment — rebuild it or restore the pinned one — or the pin is stale: an unpinned load reports the resolved digest on its handle, which is the value to commit. A `path` or `release` load whose own `pin` misses loads nothing and answers a `conflict` instead, which `Plugins` reports as `... resolved to sha256:..., not the pinned ...`.

### `... is pre-compiled and is read from unpinned ... while guests run: pin its digest`

A `[[guest]]` with a `source.path` resolved to `omnia compile` output at its first use, and the entry carries no `digest`. A manifest file's path is read while guests are already running, and a pre-compiled artifact is native code, so the runtime admits one from a path only when the entry pins the bytes (`digest = "sha256:…"`, the value the artifact hashes to). Pin the entry, or ship the guest as raw `.wasm`, which the host compiles itself. Embedded bytes (the macro's `path:`) and the component `run <component>` names are unaffected: they were read before any guest ran. Two related refusals: `... is pre-compiled, but a package admits raw wasm alone` is a `source.package` guest whose registry served `omnia compile` output — a package is admitted as raw wasm alone, however it hashes, so publish the `.wasm`; ``the bytes are a pre-compiled artifact; `Verified::wasm` admits raw wasm alone`` is a `path` or `release` a guest named itself, or an embedder's `Runtime::register`, both of which take raw wasm only — an artifact your own build produced goes through `Runtime::admit` with `Verified::trusted`.

### `Address already in use` on startup

Another process holds the trigger port. `HTTP_ADDR` (default `0.0.0.0:8080`) and `WEBSOCKET_ADDR` (default `0.0.0.0:80` — a privileged port; set it explicitly on dev machines) control the bindings.

### A backend fails to connect at startup

Backends connect eagerly during `Runtime::new`; a bad `REDIS_URL`/`POSTGRES_URL`/etc. fails the whole process by design. The error names the backend. For local work, either start the service or switch the runtime back to the in-tree default backend.

### `transport ... is not yet implemented`

Only `in-process` is a valid `[transport] default`. Remove `unix`/`nats`/`quic` from the manifest — they're reserved for distributed dispatch.

### `guest ... is the package ..., but this runtime was built without the loader feature`

A manifest declares a `source.package` guest, but the runtime has no registry client to fetch it with. Packages are fetched through omnia's `loader` feature; enable it on the `omnia` dependency (`features = ["loader"]`), or give the guest a `source.path`. The run-time twin, `... has no registry to fetch it from`, is the same gap reached through an embedder's own `RuntimeParts` with `packages: None`.

### `guest ... imports ..., but its grants reach no ...: add ... to its interfaces`

The guest declares grants with an `interfaces` list, and its component imports a package outside it. Add the package the message names. A guest built on `omnia-sdk` imports `omnia:otel` beside the WASI packages it uses, so `["wasi:*", "omnia:otel", ..]` is the usual start. An import with no function and no resource needs no entry. A guest loaded at boot is refused at startup, a first-use guest at its first use, and a loaded guest at its `load`, as `denied`.

### `guest ... has invalid grants` / `` `[plugins.grants]` is invalid ``

A grants table holds an entry of the wrong shape, and the message names it:

- an `env` entry that is not a variable name, or a prefix of one ending in `*`;
- a `mounts` entry that names no `[[mount]]` of the deployment;
- a `hosts` entry that is an IP literal, or carries a scheme, path, or port, where a host name or `*.suffix` belongs;
- an `interfaces` entry that names an interface (`wasi:http/types`) or a version (`wasi:http@0.3.0`) where a package (`wasi:http`) belongs.

### `guest declares no grants: it reaches every variable, mount, host name, guest, and interface of the deployment`

Not an error. Startup logs it at `info` for each guest without a grants table, and logs ``the deployment declares no `[plugins.grants]` `` for a loader without one, so an operator sees what nothing narrows. Declare `[guest.grants]` (or `[plugins.grants]`) to narrow it — see the [security model](security-model.md#per-guest-grants).

### `guest ... targets WASI 0.2 (wasm32-wasip2), importing ...; rebuild it for wasm32-wasip3`

The guest was built for WASI 0.2, and the runtime serves WASI 0.3 alone. Rebuild it with `--target wasm32-wasip3` against an `omnia-sdk` of the runtime's release. A compiled-in guest is refused at startup, and a declared or dynamically registered one at its first use.

### The deployment starts but a guest never runs

A guest declared in a manifest file loads at its first use, and only something naming it is a use: a `routes.*` entry matching an event, the `command = true` mark, a link call from another guest, a host dispatch, or a `loader.load(declared(..))`. An entry with none of those is never reached — it is not the catch-all, which is drawn from the guests loaded at boot alone (embedded `path:` bytes, `run <component>`). Give it routes or the `command` mark. Startup validates the manifest but does not read the file, so a wrong `source.path` also surfaces only here.

### A first-use guest fails on its first request, run, or call

Whatever would have failed at boot for a compiled-in guest fails at first use for a declared one: an unreadable `source.path`, a pin miss, a pre-compiled path without a `digest`, a package no registry routes or that resolves to pre-compiled bytes, a routed target that does not export the trigger's handler. An HTTP trigger answers `500` and logs the cause at `error`; messaging and websocket drop the event and log it; the command drive and a link call fail with the cause in the error chain; a `load` returns it typed. Fix the entry and use the guest again — nothing is cached from the failure. The first use of a raw `.wasm` guest also pays its compile, so a hot path that cannot afford it should ship a pinned `omnia compile` artifact, or embed the guest.

### The manifest loads but paths don't resolve

Manifest-relative resolution: `source.path` and `[[mount]] path` in a manifest *file* resolve against the **manifest's directory**, not the working directory. CLI `--mount` paths and relative paths in a programmatically built `Manifest` resolve against the working directory. Mixing the two is the usual cause of "file not found" after a `cd`.

## Running guests

### The guest times out (`epoch deadline` / invocation aborted around 30s)

`GUEST_TIMEOUT_MS` (default 30000) caps each **server** invocation and each link-dispatch hop on a server-rooted chain. Raise it for legitimately long request work. A command-mode (`wasi:cli/run`) chain has no wall-clock cap — neither the run itself nor the link dispatches it makes — so if a CLI guest appears to time out, check a backend timeout (for example `CURSOR_TIMEOUT_SECS`) or an external process deadline.

### `cannot block a synchronous task before returning`

A guest export typed `func` in its WIT suspended before it returned. On `wasm32-wasip3`, `std::thread::sleep` and every async-typed import suspend the task, and a sync-typed export may not. Declare the export `async func` and implement it as an `async fn`. The exports behind the `export!` macros are already async-typed; the trap comes from a raw export, such as an interface one guest serves to another.

### The guest traps growing memory

`MAX_MEMORY_BYTES` (default 256 MiB) caps linear memory. If you raise it under pooling, ensure `POOL_MAX_MEMORY_BYTES` covers the new ceiling too.

### `preopens.get_directories()` returns an empty list

No mount reached the guest. Check: a `[[mount]]` in the manifest or a `--mount` flag exists; the guest-visible `name` matches what the guest looks for (default `.`); a guest that declares grants lists the mount in its `mounts`; and with both manifest and CLI mounts, remember CLI entries override manifest entries with the same name (last wins).

A host variable a guest expects is missing for the same reasons: `[environment] inherit` or `--env` must pass it, and a guest that declares grants must list it in its `env`.

### An outbound request fails with `http-request-denied`

The guest declares grants with a `hosts` list, and the request's host matches no entry, or a redirect led to a host that matches none. Add the host name, or `*.suffix` for a domain and every host beneath it. An IP literal never matches, so call the host by its name. An entry names no port, so a granted name is reached on any port.

### `guest ... may not call ... (call to ...): its grants reach no such guest`

A guest under grants called an interface whose exporter is a guest outside its `guests` list. Add the target's name, or a prefix ending in `*` for a namespace of loaded guests (`acme:*`).

### A `load` is `denied` by the loading guest's grants

The guest that loads declares grants, and the load reaches past them. The message says which list:

- ``the loading guest's grants reach no guest `...` `` — the name the source derives is outside its `guests`;
- ``path `...` is beneath the mount `...`, which the loading guest's grants do not reach`` — the mount is outside its `mounts`;
- ``registry `...` is a host the loading guest's grants do not reach`` — the registry the load names is outside its `hosts`;
- ``guest `...` is active under other grants than this load would run it under`` — another loader admitted the guest under other grants, so this load cannot attest it.

### Guest writes to a mount fail

Mounts are **read-only by default**. Add `writable = true` (manifest) or `,writable` (CLI spec).

### Outbound HTTP or spawned work inside a handler deadlocks

Almost always a `wit-bindgen` version mismatch between your guest's dependencies and the workspace's pinned version: tasks spawned by `wasip3` land in a different executor queue than the one running. Align your guest's `wit-bindgen`/`wasip3` versions with the workspace `Cargo.toml`.

### `MAX_DISPATCH_DEPTH` exceeded

Host-mediated guest-to-guest calls are nested more than 8 deep — usually accidental recursion (guest A's import dispatches to guest B, which calls back into A). Break the cycle, or raise `MAX_DISPATCH_DEPTH` if the depth is intentional.

## Direct-command binaries

### `mybin run ...` fails with an argument error / `--manifest` is rejected

A `runtime!` binary with `mode: command` and a compiled-in deployment is a [direct command](reference/runtime-macro.md#direct-commands-raw-argv-passthrough) with **no host CLI**: no `run` subcommand, no `--manifest`/`OMNIA_MANIFEST` override, no positional wasm path. Its argv goes to the guest verbatim, so `mybin greet Ada` is correct and `mybin run -- greet Ada` hands the guest a literal `run` argument. The deployment is fixed at compile time (the compiled-in manifest), by design.

### A direct-command binary logs nothing (or too much)

One filter governs the host console and every guest: its bare level is a `-v`/`-q` flag in argv, else `RUST_LOG`'s, else the command-mode default `info`, and `RUST_LOG`'s targeted directives (`my_sdk=debug`, `tower=off`) apply on top whichever decided the level (see [Verbosity flags](reference/configuration.md#verbosity-flags)). On the console alone, the runtime's own crates (`omnia`, `omnia_core`, `omnia_link`, `omnia_plugin`) sit one rung below the command's level — a server's ladder, in either mode — so `-v` is where the runtime's `guest loaded` and `wasi:cli/run exited` lines appear and `-vv` where its `debug` does; `RUST_LOG=omnia_core=debug` names one directly under any flag. Argv still reaches the guest verbatim, so a guest whose grammar does not flatten `omnia_sdk::command::Verbosity` rejects `-v` as an unknown flag *after* the host has applied the level — declare the flags. A bare run that logs too much has a process `RUST_LOG=debug` (or similar) set: `-q` steps the bare level down and keeps the variable's targets, as does any flag; a run that is still loud under `-q` has a *targeted* directive in `RUST_LOG` doing the talking, which no flag touches. Guest tracing is the guest's own subscriber reading the `RUST_LOG` its WASI environment carries — the decided filter — and nothing in the guest changes it at run time.

## Model completions

### `unsupported: the default echo backend …`

The runtime is serving `wasi-model` with the echo `ModelDefault`, which answers text/json completions with the prompt itself and answers `unsupported` for what an echo cannot honour: a `format::schema` (it cannot fabricate a value conforming to a guest-supplied JSON Schema) or a set `generation` field. Bind a real backend (`omnia-genai`, `omnia-cursor`) in `runtime!`, or define an inline canned `ModelClient` in tests (see [Testing Policy](guides/testing-policy.md#scenario-backends)).

### `invalid` errors

The host rejected the request, or the guest's side of its session: messages with no text, a guest tool named after a reserved name (`read`, `list`, or `write`), a tool name given twice, a tool `input-schema` that is not valid JSON, a `format` schema that is not JSON, or a tool result or check correction over the host's per-result size cap. The message names the violation. A `Question<T>` also reports `invalid` when a candidate never deserialized as `T` and nothing was accepted — the steering schema and the type disagree.

### `denied: worker egress ... reaches hosts the guest's hosts grant does not`

The guest declares grants with a `hosts` list, and a worker grant's `egress` entry reaches hosts the list does not cover. An entry is covered whole or not at all: under `*.npmjs.org`, `registry.npmjs.org` and `*.cdn.npmjs.org` are covered, and `*.org` is not. Narrow the entry, or widen the guest's `hosts`.

### `no local tree on this node` (cursor backend)

The cursor backend requires a workspace: the host must mount a directory (`[[mount]]`/`--mount`) *and* the guest must lend it via `grants.workspace`. Absent either half, the spawn is refused.

## Compile / AOT

### A pre-compiled `.bin` fails to load

Compile-affecting options must match between `compile` and `run`: `MAX_FUEL`, `MEMORY_RESERVATION`, `MEMORY_GUARD_SIZE`, `BRANCH_HINTING`, `DEBUG_SYMBOLS`, `GENERATE_ADDRESS_MAP`. The compiler takes them as an explicit `CompileOptions` — `CompileOptions::default()` is the runtime's environment defaults — so check what the compile passed against what the runtime's environment sets, and recompile with the production values or align the runtime's environment with them. The artifact must also be for the runtime's target: a compile that named no `target` produced an artifact for the machine it ran on.

### `compile` prints an error from my `runtime!` binary

The generated `main` handles only `run`. Expose compilation via a hand-written `main` that calls `omnia::compile` — see the [CLI reference](reference/cli.md#compile-jit-feature).
