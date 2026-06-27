# CLI Command Example

A `wasi:cli/command` guest — the *command* shape, in contrast to every other
example, which is a *reactor* (a `cdylib` exporting a handler the host drives on
each inbound event). This guest is a plain binary that exports `wasi:cli/run`;
the host invokes it **once** and exits with its status.

## What it shows

- [`guest.rs`](guest.rs) is a Rust **binary** (no `crate-type`) that the
  `wasm32-wasip2` target maps onto the `wasi:cli/run` export — no `wit-bindgen`.
  It dispatches on argv:
  - `greet [name]` — prints `Hello, <name>!` (default `world`).
  - `add [n...]` — prints the sum of its integer arguments.
  - `env` — prints the inherited environment, one `key=value` per line.
  An unknown subcommand exits nonzero.
- [`runtime.rs`](runtime.rs) is a small hand-written host (modelled on
  `crates/omnia/tests/linking.rs`). It loads the guest through the `omnia`
  registry pipeline, **injects argv** into the per-store WASI context, and
  invokes `wasi:cli/run` through Wasmtime's typed `CommandPre` bindings.

A hand-written host is used because the floor has no `wasi:cli/run` invoker
(every trigger is a long-lived `Server`) and `StoreBase` wires env + stdio but
never sets guest argv. Both gaps are closed locally in `runtime.rs` with no
floor change.

## Build the guest

```bash
cargo build --example cli-wasm --target wasm32-wasip2
```

This emits `target/wasm32-wasip2/debug/examples/cli-wasm.wasm`. Note the binary
keeps its hyphen: unlike the `cdylib` guests (whose crate name forces
`*_wasm.wasm`), a binary example's component is named after the target as-is.

## Run

Unlike the reactor examples, this host is not the `omnia` CLI: it parses its own
argv, so there is **no `run` subcommand**. The first argument is the component
path; everything after it is forwarded to the guest as its argv.

```bash
cargo run --example cli -- ./target/wasm32-wasip2/debug/examples/cli-wasm.wasm greet Ada
cargo run --example cli -- ./target/wasm32-wasip2/debug/examples/cli-wasm.wasm add 2 3 4
cargo run --example cli -- ./target/wasm32-wasip2/debug/examples/cli-wasm.wasm env
```

Expected output:

```text
Hello, Ada!
9
<the host environment, one KEY=value per line>
```
