# Troubleshooting

Common failures and their fixes, grouped by when they bite. If your problem isn't here, run with `RUST_LOG=debug` — the runtime logs its decisions (manifest resolution, mount layering, backend connection, routing) at `debug`.

## Building

### `can't find the wasm file` / the guest built with the wrong name

Guest binaries use **underscores**, even when the cargo target uses hyphens: `cargo build --example http-wasm` produces `http_wasm.wasm`, not `http-wasm.wasm`. Check `target/wasm32-wasip2/debug/examples/` for the actual name.

### Building the whole workspace for `wasm32-wasip2` fails

Expected. Native host crates (wasmtime, tokio, backends) don't compile for the wasm target. Build guest targets explicitly:

```bash
cargo build --example <name>-wasm --target wasm32-wasip2
```

never `cargo build --workspace --target wasm32-wasip2`.

### `error[E0463]: can't find crate for 'wasip3'` on a native build

The `wasip3` crate exists only for the wasm target. Guest modules must be gated:

```rust
#![cfg(target_arch = "wasm32")]
```

and shared crates put wasm-only dependencies under `[target.'cfg(target_arch = "wasm32")'.dependencies]`.

### `cargo nextest` fails to build or behaves oddly

Install it with `--locked`:

```bash
cargo install --locked cargo-nextest
```

### `cargo fmt` produces unexpected diffs or errors

Formatting uses nightly rustfmt: `cargo +nightly fmt --all`. The stable formatter doesn't understand the workspace's `rustfmt.toml` options.

## Starting the host

### The host prints nothing and appears hung

It's probably running fine — startup logs are at `info` and off by default. Set `RUST_LOG=info` and look for the `omnia ready` line. Without it, the only output is Cargo's `Running ...`.

### `no guest specified: pass a <wasm> path, or --manifest <omnia.toml>`

The `run` subcommand needs either a positional `.wasm`/`.bin` path or a manifest via `--manifest`/`OMNIA_MANIFEST`. Also check argument order: flags for the *host* go before `--`, guest argv after it.

The embedder-path variant — `no deployment manifest supplied and OMNIA_MANIFEST is unset` — means a `DeploymentBuilder` was built without `.manifest(...)` and no `OMNIA_MANIFEST` fallback was available.

### `no guest ... is declared by this deployment`

A guest called `omnia:plugins/loader.load` with a `declared` name the manifest does not declare. A `declared` load names only `[[guest]]` entries (the macro's `guests:`); add one for the name — a manifest file's entry loads at that first `load` — or, for a component the deployment does not declare, have the guest name its `path` beneath a read-only mount or its `registry` package instead. A runtime built without omnia's `loader` feature has no loader at all: a guest importing it fails at instantiation, and a manifest declaring `registries` or a `source.package` guest is refused at startup until the feature is enabled.

### `no registry routes ...`

A `source.package` guest (the macro's `package:`) was first used, or a guest named a `registry` package of its own, but the `registries` configuration (`registries: include_str!("wasm-pkg.toml")` in the macro, `[registries] path` in a manifest) is absent or names neither a `default_registry` nor the package's namespace. Add one, pin the package to its registry under `[package_registry_overrides]`, or — for a package a guest names — have the load name its registry `endpoint`, which serves a namespace the configuration routes nowhere.

### `... is routed to ... by the deployment's registries; it cannot be fetched from ...`

A guest named a `registry` package with an `endpoint` other than the registry the deployment's `registries` routes its namespace to. The deployment's routing outranks the load's: drop the endpoint, or route the namespace to that registry in the configuration.

### `path ... is beneath the writable mount ...`

A guest named a component `path` beneath a mount the deployment marks `writable`. A component loads from a read-only mount alone — what a guest can write, it cannot run. Mount the code directory read-only and keep the guest's state under a mount of its own; the two may not share or nest directories (`writable mount ... shares its directory with the read-only mount ...` at startup, judged by directory identity, so a bind mount or firmlink of the code directory counts as it).

### `... would register as ..., a guest this deployment declares`

A guest named a `path` or `registry` package whose derived name — the path's file stem, the package reference without its version — is a `[[guest]]` the deployment declares, whether or not that guest has loaded yet. A declared name is bound by its entry alone, so nothing a caller names can seat other bytes under it: load the guest as `declared(name)`, or rename the file or the entry so the two no longer collide.

### `... resolved to sha256:..., not its declared digest ...`

The guest's bytes do not hash to the `digest` its `[[guest]]` entry pins, or the one a `path` or `registry` load carried. For a guest loaded at boot (embedded bytes, `run <component>`) this fails startup; for a first-use guest it fails the use that named it — the request, the run, the link call, or the `load`. Either the artifact changed under the deployment — rebuild it or restore the pinned one — or the pin is stale: an unpinned load reports the resolved digest on its handle, which is the value to commit.

### `... is pre-compiled and is read from unpinned ... while guests run: pin its digest`

A `[[guest]]` with a `source.path` resolved to `omnia compile` output at its first use, and the entry carries no `digest`. A manifest file's path is read while guests are already running, and a pre-compiled artifact is native code, so the runtime admits one from a path only when the entry pins the bytes (`digest = "sha256:…"`, the value the artifact hashes to). Pin the entry, or ship the guest as raw `.wasm`, which the host compiles itself. Embedded bytes (the macro's `path:`) and the component `run <component>` names are unaffected: they were read before any guest ran. Two related refusals: `... is pre-compiled, but a package admits raw wasm alone` is a `source.package` guest whose registry served `omnia compile` output — a package is admitted as raw wasm alone, however it hashes, so publish the `.wasm`; ``the bytes are a pre-compiled artifact; `Verified::wasm` admits raw wasm alone`` is a `path` or `registry` location a guest named itself, or an embedder's `Runtime::register`, both of which take raw wasm only — an artifact your own build produced goes through `Runtime::admit` with `Verified::trusted`.

### `Address already in use` on startup

Another process holds the trigger port. `HTTP_ADDR` (default `0.0.0.0:8080`) and `WEBSOCKET_ADDR` (default `0.0.0.0:80` — a privileged port; set it explicitly on dev machines) control the bindings.

### A backend fails to connect at startup

Backends connect eagerly during `Runtime::new`; a bad `REDIS_URL`/`POSTGRES_URL`/etc. fails the whole process by design. The error names the backend. For local work, either start the service or switch the runtime back to the in-tree default backend.

### `transport ... is not yet implemented`

Only `in-process` is a valid `[transport] default`. Remove `unix`/`nats`/`quic` from the manifest — they're reserved for distributed dispatch.

### `guest ... is the package ..., but this runtime was built without the loader feature`

A manifest declares a `source.package` guest, but the runtime has no registry client to fetch it with. Packages are fetched through omnia's `loader` feature; enable it on the `omnia` dependency (`features = ["loader"]`), or give the guest a `source.path`. The run-time twin, `... has no registry to fetch it from`, is the same gap reached through an embedder's own `RuntimeParts` with `packages: None`.

### The deployment starts but a guest never runs

A guest declared in a manifest file loads at its first use, and only something naming it is a use: a `routes.*` entry matching an event, the `command = true` mark, a link call from another guest, a host dispatch, or a `loader.load(declared(..))`. An entry with none of those is never reached — it is not the catch-all, which is drawn from the guests loaded at boot alone (embedded `path:` bytes, `run <component>`). Give it routes or the `command` mark. Startup validates the manifest but does not read the file, so a wrong `source.path` also surfaces only here.

### A first-use guest fails on its first request, run, or call

Whatever would have failed at boot for a compiled-in guest fails at first use for a declared one: an unreadable `source.path`, a pin miss, a pre-compiled path without a `digest`, a package no registry routes or that resolves to pre-compiled bytes, a routed target that does not export the trigger's handler. An HTTP trigger answers `500` and logs the cause at `error`; messaging and websocket drop the event and log it; the command drive and a link call fail with the cause in the error chain; a `load` returns it typed. Fix the entry and use the guest again — nothing is cached from the failure. The first use of a raw `.wasm` guest also pays its compile, so a hot path that cannot afford it should ship a pinned `omnia compile` artifact, or embed the guest.

### The manifest loads but paths don't resolve

Manifest-relative resolution: `source.path` and `[[mount]] path` in a manifest *file* resolve against the **manifest's directory**, not the working directory. CLI `--mount` paths and relative paths in a programmatically built `Manifest` resolve against the working directory. Mixing the two is the usual cause of "file not found" after a `cd`.

## Running guests

### The guest times out (`epoch deadline` / invocation aborted around 30s)

`GUEST_TIMEOUT_MS` (default 30000) caps each **server** invocation and each link-dispatch hop on a server-rooted chain. Raise it for legitimately long request work. A command-mode (`wasi:cli/run`) chain has no wall-clock cap — neither the run itself nor the link dispatches it makes — so if a CLI guest appears to time out, check a backend timeout (for example `CURSOR_TIMEOUT_SECS`) or an external process deadline.

### The guest traps growing memory

`MAX_MEMORY_BYTES` (default 256 MiB) caps linear memory. If you raise it under pooling, ensure `POOL_MAX_MEMORY_BYTES` covers the new ceiling too.

### `preopens.get_directories()` returns an empty list

No mount reached the guest. Check: a `[[mount]]` in the manifest or a `--mount` flag exists; the guest-visible `name` matches what the guest looks for (default `.`); and with both manifest and CLI mounts, remember CLI entries override manifest entries with the same name (last wins).

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

One level governs the host console and every guest: a `-v`/`-q` flag in argv, else `RUST_LOG`, else the command-mode default `info` (see [Verbosity flags](reference/configuration.md#verbosity-flags)). Argv still reaches the guest verbatim, so a guest whose grammar does not flatten `omnia_sdk::api::command::Verbosity` rejects `-v` as an unknown flag *after* the host has applied the level — declare the flags. A bare run that logs too much has a process `RUST_LOG` set: `-q` overrides it, as does any flag. Guest tracing is the guest's own subscriber reading the `RUST_LOG` its WASI environment carries — the decided level, whichever of the three sources decided it — and nothing in the guest changes it at run time.

## Model completions

### `the default echo backend cannot satisfy format::schema`

The runtime is serving `wasi-model` with the echo `ModelDefault`, which answers text/json completions with the prompt itself but cannot fabricate a value conforming to a guest-supplied JSON Schema. Bind a real backend (`omnia-genai`, `omnia-cursor`) in `runtime!`, or define an inline canned `WasiModelCtx` in tests (see [Testing Policy](guides/testing-policy.md#canned-model-backends)).

### `invalid-request` errors

The host rejected the request before any backend ran: empty `messages`, a guest tool named after a reserved name (`read`, `list`, `write`, `check`), function-tool `parameters` that are not valid JSON, or a `format` schema that is not JSON. The message names the violation. A `Question<T>` also reports `InvalidRequest` when a candidate never deserialized as `T` and nothing was accepted — the steering schema and the type disagree.

### `no local tree on this node` (cursor backend)

The cursor backend requires a workspace: the host must mount a directory (`[[mount]]`/`--mount`) *and* the guest must lend it via `grants.workspace`. Absent either half, the spawn is refused.

### MCP tools rejected (`genai` backend)

`Tool::Mcp` grants are only supported by the cursor backend. Use `omnia-cursor`, or restrict the request to `Tool::Function` declarations.

## Compile / AOT

### A pre-compiled `.bin` fails to load

Compile-affecting options must match between `compile` and `run`: `MAX_FUEL`, `MEMORY_RESERVATION`, `MEMORY_GUARD_SIZE`, `BRANCH_HINTING`, `DEBUG_SYMBOLS`, `GENERATE_ADDRESS_MAP`. The compiler takes them as an explicit `CompileOptions` — `CompileOptions::default()` is the runtime's environment defaults — so check what the compile passed against what the runtime's environment sets, and recompile with the production values or align the runtime's environment with them. The artifact must also be for the runtime's target: a compile that named no `target` produced an artifact for the machine it ran on.

### `compile` prints an error from my `runtime!` binary

The generated `main` handles only `run`. Expose compilation via a hand-written `main` that calls `omnia::compile` — see the [CLI reference](reference/cli.md#compile-jit-feature).
