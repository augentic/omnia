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

### `a server runs no command, so it takes no arguments: ... given`

The runtime is built with `mode: server`, and its command line carries arguments after `--`, or an embedder passed them to `DeploymentBuilder::args`. A server runs no `wasi:cli/run`, so nothing would receive them, and the runtime refuses to start rather than drop them. The message counts the arguments and never prints them, since one can hold a secret. Drop them, or build the runtime with `mode: command` when the guest is a command. A guest a trigger serves takes its input another way; see [A guest's `get-arguments` is empty](#a-guests-get-arguments-is-empty).

### `no guest ... is declared by this deployment`

A guest called `omnia:plugins/loader.load` with a `declared` name the manifest does not declare. A `declared` load names only `[[guest]]` entries (the macro's `guests:`); add one for the name — a manifest file's entry loads at that first `load` — or, for a component the deployment does not declare, have the guest name its `path` beneath a read-only mount or its `release` instead. A runtime built without omnia's `loader` feature has no loader at all: a guest importing it fails at instantiation, and a manifest declaring `plugins` or a `source.package` guest is refused at startup until the feature is enabled.

### `no registry routes ...`

A `source.package` guest (the macro's `package:`) was first used, or a guest named a `release` of its own; the deployment's store holds no release under that reference, and the `registries` configuration (`plugins: { registries: include_str!("wasm-pkg.toml") }` in the macro, `[plugins] registries.path` in a manifest) is absent or names neither a `default_registry` nor the package's namespace. The refusal names the store file that would have served the release: fetch it there (`wkg get <reference> -o <store>/`) or copy a component under that name, route the namespace, pin the package to its registry under `[package_registry_overrides]`, or — for a release a guest names — have the load name its `registry`, which serves a namespace the configuration routes nowhere (and is never written to the store, so every such load fetches).

### `... is routed to ... by the deployment's registries; it cannot be fetched from ...`

A guest named a `release` with a `registry` other than the one the deployment's `registries` routes its namespace to. The deployment's routing outranks the load's: drop the registry, or route the namespace to it in the configuration.

### `registry ... is a host the loading guest's grants do not reach`

A guest named a `release` with a `registry` whose host its `hosts` grant does not reach, so the load is `denied` before anything is fetched. Add the registry's host to the loading guest's `hosts`, or drop the registry and let the deployment's routing pick one. A registry spelled as an address, or ending in a `.`, is refused under any list, `*` included. A `registry` that is not a host name with an optional `:port` is `invalid`, as `registry ... is not a valid name`, whatever the grants.

### `registry ... leads to ..., a host the loading guest's grants do not reach`

A guest named a `release` with a `registry` its `hosts` grant reaches, and the fetch led to a host the grant does not: the OCI host the registry's metadata names, the token realm a `401` names, or a redirect. The load is `denied` before anything connects to that host. One registry often spans several hosts. `omnia.host` redirects its metadata to `augentic.io`, which names `ghcr.io`, and `ghcr.io` redirects each layer to `pkg-containers.githubusercontent.com`, so a guest that loads from `omnia.host` lists all four. To list none, route the namespace in the deployment's `registries` and drop the load's `registry`: the registry the deployment's routing picks is held to the routing alone.

### `the loading guest's grants reach no guest ..., which ... names`

A guest called `omnia:plugins/loader.load`, and its grants table's `guests` list leaves out the name the source registers under: the declared name, a path's file stem, or a release's reference without its version. The load is `denied` before anything is read or fetched, whatever the source. A table that leaves out `guests` reaches no guest. Add the name, or a prefix of it that ends in `*`, to the loading guest's `guests`. A loading guest that was itself loaded by path or release runs under `[plugins.grants]` within its own loader's grants, so add the name to both.

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

Backends connect eagerly at startup, before the deployment assembles; a bad `REDIS_URL`/`POSTGRES_URL`/etc. fails the whole process by design. The error names the backend. For local work, either start the service or switch the runtime back to the in-tree default backend.

### `transport ... is not yet implemented`

Only `in-process` is a valid `[transport] default`. Remove `unix`/`nats`/`quic` from the manifest — they're reserved for distributed dispatch.

### `guest ... is the package ..., but this runtime was built without the loader feature`

A manifest declares a `source.package` guest, but the runtime has no registry client to fetch it with. Packages are fetched through omnia's `loader` feature; enable it on the `omnia` dependency (`features = ["loader"]`), or give the guest a `source.path`.

### `guest ... imports code from packages its grants do not reach: add ... to its interfaces`

The guest declares grants, and its component imports code from packages its `interfaces` list leaves out. The message names every one, so add them all at once. A guest built on `omnia-sdk` imports `wasi:cli`, `wasi:clocks`, `wasi:random`, and `omnia:otel`, so a list starts with those four. Name each package rather than writing `wasi:*`, which would grant `wasi:http` and `wasi:sockets` as well. An import with no function and no resource needs no entry. A guest loaded at boot is refused at startup, and any other guest at its first use, as `denied`.

A guest loaded by path or release is refused at its load, as `denied`, and the message adds that it runs under `[plugins.grants]` within the loading guest's grants. Such a guest has no table of its own, so add the package to the loading guest's `interfaces`, and to `[plugins.grants]` where the deployment declares one.

### `guest ... has invalid grants`

A grants table holds an entry the deployment cannot honour, and the message names it:

- an `env` entry that is not a variable name, or a prefix of one ending in `*`;
- an `env` entry that reaches no variable `[env] inherit` or `--env` passes;
- a `mounts` entry that names no `[[mount]]` of the deployment;
- a `hosts` entry that carries a scheme, a port, or a path, is an IP literal, or ends in a numeric label (`2130706433`, `example.1`), which reads as an IP address, where a host name, `*.suffix`, or `*` belongs;
- a `guests` entry that is empty or holds a `*` before its end, where a guest name or a prefix of one ending in `*` belongs;
- an `interfaces` entry that names an interface (`wasi:http/types`), a version (`wasi:http@0.3.0`), or a bare namespace (`wasi`), where a package (`wasi:http`) or a prefix such as `wasi:*` belongs.

Startup refuses a manifest's table, and refuses `[plugins.grants]` for the same entries as `` `[plugins.grants]` is invalid ``. `Runtime::register` and `Runtime::admit` refuse an embedder's as `invalid`, and `Deployment::assemble` refuses the loader's.

### `guest ... is active under other grants than this admission names`

An embedder's `Runtime::register`, or a guest's path or release load, named a guest already active under other grants. A name already active attests on the same bytes and the same grants alone, so a caller never believes narrower grants hold a guest that keeps its first ones. Register under the grants the guest runs under, or under another name. `Runtime::admit` never attests: it answers `Conflict` for any name already taken.

Two loaders under different grants reach it when both load the same path or release. The first load seats the guest under `[plugins.grants]` within its loader's grants, and the second names that table within its own. Declare a `[plugins.grants]` both loaders' grants cover, so both loads narrow to the table itself.

### `guest declares no grants: it reaches every variable, mount, and interface of the deployment`

Not an error. Startup logs it for each guest without a grants table, at `info` while no guest declares grants and at `warn` once one does, so an operator sees what nothing narrows. Declare `[guest.grants]` to narrow it; see the [security model](security-model.md#per-guest-grants).

### `the deployment declares no [plugins.grants]: a guest loaded at run time reaches whatever its loader does`

Not an error. A runtime built with the `loader` feature logs it at `info` when the deployment declares no `[plugins.grants]`. A guest loaded by path or release then runs under the grants of the guest that loaded it. Declare `[plugins.grants]` to narrow such guests further; see the [security model](security-model.md#guest-requested-plugin-loading-omniapluginsloader).

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

### The HTTP trigger answers `400 Bad Request` before any guest runs

The request's target is malformed. The trigger refuses a request that:

- carries no `Host` header, unless it is HTTP/2 with an `:authority`;
- carries more than one `Host`, or one that does not parse;
- names an authority with userinfo (`user@host`) or a non-numeric port (`host:http`), in its `Host`, its URI, or a `Forwarded` element's `host=`;
- is HTTP/2 with an `:authority` other than its `Host`.

An HTTP/1.1 request with an absolute-form URI still needs a `Host`. A conforming client always sends one, so the usual cause is a hand-built request, such as one a test passes to `HttpHandler::handle`. `omnia_test::host::send` sets a missing `Host` from the URI. The trigger logs each refusal at `debug` as `rejecting malformed request`, with its reason.

### The HTTP trigger answers `503 Service Unavailable` with `Retry-After: 1`

The trigger refused the request before its guest ran, for one of two reasons.

Every slot of the [concurrency gate](reference/configuration.md#concurrency-gate) is held, so the trigger refused the request before it instantiated a guest. It counts each refusal on `invocations_refused`. A WebSocket event is refused at the same point, and fails naming `MAX_CONCURRENT_INVOCATIONS`. Message deliveries waiting for a slot are served first, which `invocations_waiting` shows. No more than `MESSAGING_MAX_IN_FLIGHT` hold or wait for one, and lowering it leaves HTTP more of the gate. Raise `POOL_MAX_INSTANCES`, and `MAX_CONCURRENT_INVOCATIONS` with it where it is set, if the host has the memory. A probe of `HTTP_HEALTH_PATH` takes no slot, so it still answers `200`.

Or the request took a slot, but the instance pool had no room for its guest. The trigger logs `the instance pool is full` at `warn`, naming the guest, and does not count the request on `invocations_refused`. The [next entry](#the-instance-pool-is-full) gives the causes.

### `the instance pool is full`

An instance found no free slot of a kind it takes, which the end of the message names, such as `memory stripe 0`. An HTTP request answers `503` and logs it at `warn`. A link call and a command's run fail with it in the error chain. Its causes are:

- **Calls between guests.** A call from one guest to another takes an instance but no slot of the gate, and with `MAX_CONCURRENT_INVOCATIONS` unset the gate is as wide as the pool, so the triggers' own invocations can take every instance. Set `MAX_CONCURRENT_INVOCATIONS` below `POOL_MAX_INSTANCES` by the margin [Pool sizing](guides/performance-tuning.md#2-pool-sizing) works out, or raise `POOL_MAX_INSTANCES`.
- **A batched decommit with a set total.** Above a `POOL_DECOMMIT_BATCH_SIZE` of 1, freed slots wait to be reset before they are reused. A set `POOL_TOTAL_*` is used as is, without the [decommit margin](reference/configuration.md#instance-pooling) the unset totals add. Add the margin to it, or set the batch back to 1.
- **A total below what the guests take.** A component composed from several modules takes more memories, tables, or core instances than the [totals](reference/configuration.md#instance-pooling) allow for. Set the `POOL_TOTAL_*` it needs.

### A streamed HTTP response fails part way

The client got the head and part of the body, then the body was cut short: HTTP/1.1 closes the connection mid-body, and HTTP/2 resets the stream. Either the guest trapped while it wrote the body, which the host logs at `error`, or the body was still streaming when `HTTP_STREAM_TIMEOUT_MS` passed, which it logs at `warn` as `response body outlived HTTP_STREAM_TIMEOUT_MS`. Raise or unset the cap for a long-lived stream, such as server-sent events.

### `cannot block a synchronous task before returning`

A guest export typed `func` in its WIT suspended before it returned. On `wasm32-wasip3`, `std::thread::sleep` and every async-typed import suspend the task, and a sync-typed export may not. Declare the export `async func` and implement it as an `async fn`. The exports behind the `export!` macros are already async-typed; the trap comes from a raw export, such as an interface one guest serves to another.

### The guest traps growing memory

`MAX_MEMORY_BYTES` (default 256 MiB) caps linear memory. If you raise it under pooling, ensure `POOL_MAX_MEMORY_BYTES` covers the new ceiling too.

### `preopens.get_directories()` returns an empty list

No mount reached the guest. Check: a `[[mount]]` in the manifest or a `--mount` flag exists; the guest-visible `name` matches what the guest looks for (default `.`); a guest that declares grants lists the mount in its `mounts`; and with both manifest and CLI mounts, remember CLI entries override manifest entries with the same name (last wins).

### A host variable is missing in the guest

A guest sees only the host variables its deployment names, beside the `RUST_LOG` and `NO_COLOR` the runtime sets. Name the variable, or a prefix of it ending in `*`, in one of:

- the manifest's `[env] inherit`;
- the `runtime!` invocation's `env: { inherit: [..] }`, the only way for a direct command;
- a `--env` flag on `run`.

A guest that declares grants sees, of these, only those its `env` list names.

Matching is case-sensitive against the name as the host spells it, so `PATH` does not pass a Windows host's `Path`. A value the guest reads through `config` is answered by the `omnia:config` backend, apart from this list; see [A config key reads as unset](#a-config-key-reads-as-unset).

### A config key reads as unset

`ConfigDefault` serves only the host variables `CONFIG_KEYS` names, or the keys the runtime compiles in, and answers any other key as unset. `config::require` and `Settings::load` then fail the key as a server error with the code `not-found`. The first read of each variable the host sets but `ConfigDefault` does not serve logs a warning naming the variable. Name it, or a prefix of it ending in `*`, in `CONFIG_KEYS`, as `CONFIG_KEYS="ORDERS_*,TZ"` does.

### A guest's `get-arguments` is empty

Only the command run receives the command's arguments (see the [security model](security-model.md#the-trust-boundary)). Every other guest sees an empty argv, whatever its grants, so it takes its input another way:

- A guest the command calls over a link, or loads through `omnia:plugins`, takes it in the call.
- A guest a trigger serves has no caller to pass it. It reads a host variable named in `[env] inherit` (see [A host variable is missing in the guest](#a-host-variable-is-missing-in-the-guest)) or a key from `omnia:config` (see [A config key reads as unset](#a-config-key-reads-as-unset)).

### Guest writes to a mount fail

Mounts are **read-only by default**. Add `writable = true` (manifest) or `,writable` (CLI spec).

### An outbound request fails with `http-request-denied`

The guest declares grants, and the request's host, or the host a redirect leads to, is outside its `hosts` list. The host refuses it before anything connects. A table that leaves out `hosts`, `[plugins.grants]` included, reaches no host at all. Add the host: a whole name, `*.suffix` for a domain and every host beneath it, or `*` for every host name. A host spelled as an address (`127.0.0.1`, `2130706433`, `[::1]`), or one ending in a `.`, is refused under any list, `*` included, so name the host instead. Only a guest without a grants table reaches an address. In a test, `MatchedHttp` refuses the same requests and records none of them.

### `guest ... may not call ... (call to ...): its grants reach no such guest`

The calling guest declares grants, and the guest its link call names is outside its `guests` list. The relay refuses the call before the target loads, so nothing of the target is read or run. A table that leaves out `guests`, `[plugins.grants]` included, reaches no guest at all. Add the target's name, or a prefix of it that ends in `*`, to the caller's `guests`; `*` reaches every guest. The caller also needs the package it calls through in its `interfaces`, or it is refused at admission instead.

### Outbound HTTP or spawned work inside a handler deadlocks

Almost always a `wit-bindgen` version mismatch between your guest's dependencies and the workspace's pinned version: tasks spawned by `wasip3` land in a different executor queue than the one running. Align your guest's `wit-bindgen`/`wasip3` versions with the workspace `Cargo.toml`.

### `MAX_DISPATCH_DEPTH` exceeded

Host-mediated guest-to-guest calls are nested more than 8 deep — usually accidental recursion (guest A's import dispatches to guest B, which calls back into A). Break the cycle, or raise `MAX_DISPATCH_DEPTH` if the depth is intentional.

## Direct-command binaries

### `mybin run ...` fails with an argument error / `--manifest` is rejected

A `runtime!` binary with `mode: command` and a compiled-in deployment is a [direct command](reference/runtime-macro.md#direct-commands-raw-argv-passthrough) with **no host CLI**: no `run` subcommand, no `--manifest`/`OMNIA_MANIFEST` override, no positional wasm path. Its argv goes to the command guest verbatim, so `mybin greet Ada` is correct and `mybin run -- greet Ada` hands the command guest a literal `run` argument. The deployment is fixed at compile time (the compiled-in manifest), by design.

### A direct-command binary logs nothing (or too much)

One filter governs the host console and every guest: its bare level is a `-v`/`-q` flag in argv, else `RUST_LOG`'s, else the command-mode default `info`, and `RUST_LOG`'s targeted directives (`my_sdk=debug`, `tower=off`) apply on top whichever decided the level (see [Verbosity flags](reference/configuration.md#verbosity-flags)). On the console alone, the runtime's own crates (`omnia`, `omnia_core`, `omnia_link`, `omnia_plugin`) sit one rung below the command's level — a server's ladder, in either mode — so `-v` is where the runtime's `guest loaded` and `wasi:cli/run exited` lines appear and `-vv` where its `debug` does; `RUST_LOG=omnia_core=debug` names one directly under any flag. Argv still reaches the command guest verbatim, so one whose grammar does not flatten `omnia_sdk::command::Verbosity` rejects `-v` as an unknown flag *after* the host has applied the level — declare the flags. A bare run that logs too much has a process `RUST_LOG=debug` (or similar) set: `-q` steps the bare level down and keeps the variable's targets, as does any flag; a run that is still loud under `-q` has a *targeted* directive in `RUST_LOG` doing the talking, which no flag touches. Guest tracing is the guest's own subscriber reading the `RUST_LOG` its WASI environment carries — the decided filter — and nothing in the guest changes it at run time.

## Model completions

### `unsupported: the default echo backend …`

The runtime is serving `wasi-model` with the echo `ModelDefault`, which answers text/json completions with the prompt itself and answers `unsupported` for what an echo cannot honour: a `format::schema` (it cannot fabricate a value conforming to a guest-supplied JSON Schema) or a set `generation` field. Bind a real backend (`omnia-genai`, `omnia-cursor`) in `runtime!`, or define an inline canned `ModelClient` in tests (see [Testing Policy](guides/testing-policy.md#scenario-backends)).

### `invalid` errors

The host rejected the request, or the guest's side of its session: messages with no text, a `worker` grant entry that breaks its shape (such as an `egress` entry that is an IP address or ends in a numeric label, like `2130706433`), a guest tool named after a reserved name (`read`, `list`, or `write`), a tool name given twice, a tool `input-schema` that is not valid JSON, a `format` schema that is not JSON, or a tool result or check correction over the host's per-result size cap. The message names the violation. A `Question<T>` also reports `invalid` when a candidate never deserialized as `T` and nothing was accepted — the steering schema and the type disagree.

### `worker egress ... reaches hosts the guest's grants do not`

The completion is `denied`: its worker `egress` names a host the guest's `hosts` grant does not cover. Each entry must lie within the list. `*.example.com` covers `api.example.com` and `*.api.example.com`, and `api.example.com` covers itself alone. Add the host to the guest's `hosts`, or narrow the egress.

### `no local tree on this node` (cursor backend)

The cursor backend requires a workspace: the host must mount a directory (`[[mount]]`/`--mount`) *and* the guest must lend it via `grants.workspace`. Absent either half, the spawn is refused.

## Compile / AOT

### A pre-compiled `.bin` fails to load

Compile-affecting options must match between `compile` and `run`: `MAX_FUEL`, `MEMORY_RESERVATION`, `MEMORY_GUARD_SIZE`, `BRANCH_HINTING`, `DEBUG_SYMBOLS`, `GENERATE_ADDRESS_MAP`. The `compile` subcommand reads them from its own environment, and the library compiler takes them as an explicit `CompileOptions` — `CompileOptions::default()` is the runtime's environment defaults — so check what the compile ran under against what the runtime's environment sets, and recompile with the production values or align the runtime's environment with them. The artifact must also be for the runtime's target: a compile that named no `target` produced an artifact for the machine it ran on.

### `` `compile` needs omnia's `jit` feature, which this runtime was built without ``

The runtime was built without omnia's default `jit` feature, so it carries no compiler. Compile with a build of the same runtime that keeps `jit`, under the settings the loading runtime runs with (see [A pre-compiled `.bin` fails to load](#a-pre-compiled-bin-fails-to-load)), and load the `.bin` here.
