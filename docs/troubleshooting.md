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

### `no guest specified: pass a <wasm> path, or --config <omnia.toml>`

The `run` subcommand needs either a positional `.wasm`/`.bin` path or a manifest via `--config`/`OMNIA_CONFIG`. Also check argument order: flags for the *host* go before `--`, guest argv after it.

### `Address already in use` on startup

Another process holds the trigger port. `HTTP_ADDR` (default `0.0.0.0:8080`) and `WEBSOCKET_ADDR` (default `0.0.0.0:80` — a privileged port; set it explicitly on dev machines) control the bindings.

### A backend fails to connect at startup

Backends connect eagerly during `Runtime::new`; a bad `REDIS_URL`/`POSTGRES_URL`/etc. fails the whole process by design. The error names the backend. For local work, either start the service (`docker compose -f docker/<service>.yaml up -d`) or switch the runtime back to the in-tree default backend.

### `transport ... is not yet implemented`

Only `in-process` is a valid `[transport] default`. Remove `unix`/`nats`/`quic` from the manifest — they're reserved for distributed dispatch.

### `guest ...: OCI source ... is not yet supported`

`source.oci` parses but isn't implemented. Use `source.path`.

### The manifest loads but paths don't resolve

Manifest-relative resolution: `source.path` and `[[mount]] path` resolve against the **manifest's directory**, not the working directory. CLI `--mount` paths resolve against the working directory. Mixing the two is the usual cause of "file not found" after a `cd`.

## Running guests

### The guest times out (`epoch deadline` / invocation aborted around 30s)

`GUEST_TIMEOUT_MS` (default 30000) caps each invocation. Raise it for legitimately long work — model completions through slow backends are the common case.

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

## Model completions

### `the default echo backend cannot satisfy format::schema`

The runtime is serving `wasi-model` with the echo `ModelDefault`, which answers text/json completions with the prompt itself but cannot fabricate a value conforming to a guest-supplied JSON Schema. Bind a real backend (`omnia-genai`, `omnia-cursor`) in `runtime!`, or inject `omnia_testkit::model::ReplayBackend` in tests.

### `no replay fixture for request`

The testkit `ReplayBackend` found no fixture whose `key_request` matches the incoming request. The match is exact on the reduced request (model, system, messages, generation, format, tools, references/verify grants — see the [fixture format](reference/model.md#replay-fixture-format)); any prompt drift misses. Check the backend loads the right fixture directory and that the fixture's content matches the request byte-for-byte in those fields.

### `invalid-request` errors

The host's validation gate rejected the request before any backend ran: empty `messages`, a guest tool named after a reserved host-injected tool (`resolve`, `read`, `list`, `write`, `verify`), or an invalid JSON Schema in `format`. The message names the violation.

### `no local tree on this node` (cursor backend)

The cursor backend requires a workspace: the host must mount a directory (`[[mount]]`/`--mount`) *and* the guest must lend it via `grants.workspace`. Absent either half, the spawn is refused.

### MCP tools rejected (`genai` backend)

`Tool::Mcp` grants are only supported by the cursor backend. Use `omnia-cursor`, or restrict the request to `Tool::Function` declarations.

## Compile / AOT

### A pre-compiled `.bin` fails to load

Compile-affecting options must match between `compile` and `run`: `MAX_FUEL`, `MEMORY_RESERVATION`, `MEMORY_GUARD_SIZE`, `BRANCH_HINTING`. Recompile with the production values set, or align the runtime's environment with the compile-time one.

### `compile` prints an error from my `runtime!` binary

The generated `main` handles only `run`. Expose compilation via a hand-written `main` that calls `omnia::compile` — see the [CLI reference](reference/cli.md#compile-jit-feature).
