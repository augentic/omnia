# CLI Command Example

A `wasi:cli/command` guest driven as a **one-shot trigger**: the host invokes
its `wasi:cli/run` export exactly once and exits with the guest's status. Unlike
the long-lived triggers (HTTP, messaging, …) that loop on a transport, a command
runs to completion — but it rides the *same* `runtime!` / `TriggerRouter` floor,
so the same guest could be driven by an inbound event tomorrow with only a
host-wiring change.

## What it shows

- [`guest.rs`](guest.rs) is a `cdylib` reactor exporting `wasi:cli/run@0.3.0`
  via `wasip3::cli::command::export!` — the same shape as every other example
  guest. It dispatches on argv (read through the `std` bridge Omnia links
  alongside p3):
  - `greet [name]` — prints `Hello, <name>!` (default `world`).
  - `add [n...]` — prints the sum of its integer arguments.
  - `env` — prints the inherited environment, one `key=value` per line.

  An unknown subcommand exits `2`; missing usage exits `1`.
- [`runtime.rs`](runtime.rs) is the whole host: a single
  `omnia::runtime!({ main: true, command: true })`. Command mode finds the sole
  command-capable guest, instantiates it through the registry pipeline, drives
  `wasi:cli/run` once, and hands back the exit status the generated `main` exits
  with.

## Build the guest

```bash
cargo build --example cli-wasm --target wasm32-wasip2
```

This emits `target/wasm32-wasip2/debug/examples/cli_wasm.wasm`. The `cdylib`
crate name forces the underscore (`cli_wasm.wasm`, not `cli-wasm.wasm`).

## Run

The host *is* the `omnia` CLI, so the guest loads through the `run` subcommand —
the same surface every example uses. Everything after `--` is forwarded to the
guest as its argv (`argv[0]` is the program name, supplied by the host):

```bash
cargo run --example cli -- run ./target/wasm32-wasip2/debug/examples/cli_wasm.wasm -- greet Ada
cargo run --example cli -- run ./target/wasm32-wasip2/debug/examples/cli_wasm.wasm -- add 2 3 4
cargo run --example cli -- run ./target/wasm32-wasip2/debug/examples/cli_wasm.wasm -- env
```

Expected output:

```text
Hello, Ada!
9
<the host environment, one KEY=value per line>
```

A nonzero subcommand sets the process exit code, demonstrating the one-shot
exit-code seam (the guest's status flows back through command mode to `main`):

```bash
cargo run --example cli -- run ./target/wasm32-wasip2/debug/examples/cli_wasm.wasm -- nope
echo $?   # 2
```
