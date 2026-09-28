# CLI Command Example

Demonstrates a `wasi:cli/command` guest on the command façade: `parse`
classifies clap argv, `Command::call` projects each verb (decode →
`Client::call` → encode) onto a `Response`, and `command!` binds `main` as
the `wasi:cli/run` export.

## Quick Start

```bash
make example cli greet Ada
make example cli add 2 3 4
make example cli env

# guest flags go after `--`, or make and mise claim them
make example cli -- --help
make example cli -- --format json greet Ada
make example cli -- bogus; echo $?           # 64 (usage)
make example cli -- fail; echo $?            # 3 (server-error)
make example cli -- fail not-found; echo $?  # 2
```

Or, more manually, for debugging:

```bash
# build the guest
cargo build --example cli-wasm --target wasm32-wasip2

# run the host (everything after `--` is the guest argv)
cargo run --example cli -- run ./target/wasm32-wasip2/debug/examples/cli_wasm.wasm -- greet Ada

# test
cargo run --example cli -- run ./target/wasm32-wasip2/debug/examples/cli_wasm.wasm -- greet Ada
...
```

## Output format

Every verb takes a global `--format text|json` (default `text`). Text goes
through the verb's render fn; JSON is the handler output, pretty-printed:

```bash
$ make example cli -- --format json greet Ada
{
  "greeting": "Hello, Ada!"
}
```

A failure is the same envelope in either format on stderr — text as
`error[<code>]: <message>`, JSON as
`{"error","message","exit-code"}`.

## Exit map

The guest never picks an exit code; the error class does. `fail [CLASS]`
returns that class from its handler so the mapping can be observed:

| Argv               | Error class      | Exit |
| ------------------ | ---------------- | ---- |
| `fail bad-request` | `BadRequest`     | 1    |
| `fail not-found`   | `NotFound`       | 2    |
| `fail` (default)   | `ServerError`    | 3    |
| `fail bad-gateway` | `BadGateway`     | 4    |
| `bogus`            | clap usage error | 64   |

Usage errors exit 64 (`EX_USAGE`), so exit 2 always means `NotFound`.
Invocation metadata is read from `CLI_REQUEST_ID`, `CLI_CORRELATION_ID`,
and `CLI_CAUSATION_ID`; a missing request id is minted.
