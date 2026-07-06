# CLI Command Example

A `wasi:cli/command` guest driven as a **one-shot trigger**: the host invokes its `wasi:cli/run` export exactly once and exits with the guest's status. Unlike the long-lived triggers (HTTP, messaging, …) that loop on a transport, a command runs to completion — but it rides the *same* `runtime!` / `TriggerRouter` runtime core, so the same guest could be driven by an inbound event tomorrow with only a host-wiring change.

## What it shows

- [`guest.rs`](guest.rs) is a `cdylib` reactor exporting `wasi:cli/run@0.3.0` via `wasip3::cli::command::export!` — the same shape as every other example guest. Its CLI is ordinary [`clap`](https://docs.rs/clap) (derive API): argv and stdout/stderr arrive through the p2 `std` bridge Omnia links alongside p3, so clap works unmodified — `--help`, `--version`, and usage errors need no hand-rolling. Subcommands:
  - `greet [NAME]` — prints `Hello, <NAME>!` (default `world`).
  - `add [N...]` — prints the sum of its integer arguments.
  - `env` — prints the inherited environment, one `key=value` per line.
  - `fail [CODE]` — exits with `CODE` via the p3 `wasi:cli/exit`, or returns `Err(())` without it (which the host maps to `1`).

  One seam nuance: the guest uses `try_parse()` rather than `parse()`, forwarding clap's `exit_code()` through the p3 `wasi:cli/exit`. `parse()`'s internal `std::process::exit` lands on the *p2* `wasi:cli/exit`, which carries only success/failure and would collapse clap's usage-error code `2` to `1`. Either way the host observes the exit as wasmtime's `I32Exit`.
- [`runtime.rs`](runtime.rs) is the whole host: a single `omnia::runtime!({ mode: command })`. Command mode finds the sole command-capable guest, instantiates it through the registry pipeline, drives `wasi:cli/run` once, and hands back the exit status the generated `main` exits with.

## Build the guest

```bash
cargo build --example cli-wasm --target wasm32-wasip2
```

This emits `target/wasm32-wasip2/debug/examples/cli_wasm.wasm`. The `cdylib` crate name forces the underscore (`cli_wasm.wasm`, not `cli-wasm.wasm`).

## Run

The host *is* the `omnia` CLI, so the guest loads through the `run` subcommand — the same surface every example uses. Everything after `--` is forwarded to the guest as its argv (`argv[0]` is the program name, supplied by the host):

```bash
export RUST_LOG=info,opentelemetry_sdk=off
cargo run --example cli -- run ./target/wasm32-wasip2/debug/examples/cli_wasm.wasm -- greet Ada
cargo run --example cli -- run ./target/wasm32-wasip2/debug/examples/cli_wasm.wasm -- add 2 3 4
cargo run --example cli -- run ./target/wasm32-wasip2/debug/examples/cli_wasm.wasm -- env
cargo run --example cli -- run ./target/wasm32-wasip2/debug/examples/cli_wasm.wasm -- --help
```

Expected output:

```text
Hello, Ada!
9
<the host environment, one KEY=value per line>
<clap-generated usage>
```

Nonzero exits flow back through command mode to `main`, demonstrating the one-shot exit-code seam:

```bash
# clap usage error, via std::process::exit -> p2 wasi:cli/exit
cargo run --example cli -- run ./target/wasm32-wasip2/debug/examples/cli_wasm.wasm -- bogus
echo $?   # 2

# explicit code, via p3 wasi:cli/exit
cargo run --example cli -- run ./target/wasm32-wasip2/debug/examples/cli_wasm.wasm -- fail 42
echo $?   # 42

# plain failure: run returning Err(()) maps to 1
cargo run --example cli -- run ./target/wasm32-wasip2/debug/examples/cli_wasm.wasm -- fail
echo $?   # 1
```

## Guest binary size

clap is the heaviest of the common Rust argument parsers. This example already trims it — `default-features = false` drops `color` (anstream/anstyle) and `suggestions` (strsim), keeping only `derive`, `error-context`, `help`, `std`, and `usage` — but per the [argparse-rosetta-rs](https://github.com/rosetta-rs/argparse-rosetta-rs) benchmarks that still leaves roughly 380 KiB of parser in the binary (~600 KiB untrimmed). If guest `.wasm` size matters (distribution, cold start), two further steps down:

- **clap builder API** instead of `derive`: same runtime machinery, but drops the proc-macro build cost and the generated derive glue.
- **[`lexopt`](https://docs.rs/lexopt)** (~37 KiB) or [`pico-args`](https://docs.rs/pico-args) (~24 KiB): an order of magnitude smaller. You hand-write the `--help` text and dispatch loop, but both handle `--opt=value` and combined short flags, and both compile cleanly to `wasm32-wasip2` — nothing about the WASI seam is clap-specific.
