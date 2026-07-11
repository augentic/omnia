# CLI Command Example

A `wasi:cli/command` guest driven as a **one-shot trigger**: the host invokes its `wasi:cli/run` export exactly once and exits with the guest's status. The guest uses `omnia_guest::api::command` to bind typed `Operation` implementations to a nested command grammar while keeping the WASI export explicit.

## What it shows

- [`guest.rs`](guest.rs) defines operations over a small shared provider, registers their Clap-derived argument types with `omnia_guest::api::command::Router`, and explicitly exports `wasi:cli/run@0.3.0` via `wasip3::cli::command::export!`. Typed `Args` convert into transport-neutral operation inputs before the `Invoker` calls each operation. The export delegates once to `omnia_guest::api::command::execute_wasi`, which preserves buffered stdout, stderr, and exact exit status. Subcommands:
  - `greet [NAME]` — prints `Hello, <NAME>!` (default `world`).
  - `add [N...]` — prints the sum of its integer arguments.
  - `env` — prints the inherited environment, one `key=value` per line.
  - `fail [CODE]` — exits with `CODE` via the p3 `wasi:cli/exit`, or returns `Err(())` without it (which the host maps to `1`).

- The router generates `--help`, `--version`, usage errors, and a `completions <SHELL>` route from the same grammar. Each route decodes arguments into an operation input and projects typed output or failure into `CommandResponse`.
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
# clap usage error, projected by the command router
cargo run --example cli -- run ./target/wasm32-wasip2/debug/examples/cli_wasm.wasm -- bogus
echo $?   # 2

# operation failure projected to an explicit code
cargo run --example cli -- run ./target/wasm32-wasip2/debug/examples/cli_wasm.wasm -- fail 42
echo $?   # 42

# plain failure: run returning Err(()) maps to 1
cargo run --example cli -- run ./target/wasm32-wasip2/debug/examples/cli_wasm.wasm -- fail
echo $?   # 1
```

## Guest binary size

`omnia_guest::api::command` uses Clap with a trimmed workspace feature set that omits color and suggestions. The router owns the command grammar and operation dispatch; the guest still owns its explicit WASI export and application-local decoding and projection policy.
