# Command Resolver Example

Demonstrates the `runtime!` deployment keys: a static guest plus a
`GuestResolver` for resolve-on-miss, explicit command routing
(`command_guest:`), and raw argv passthrough (`program:`).

Because `program:` is set, the binary has **no host CLI**: there is no `run`
subcommand and no `--config`/`OMNIA_CONFIG`/positional-wasm override — every
argument is forwarded to the guest verbatim.

## Quick Start

```bash
# build the guest this deployment compiles in
cargo build --example cli-wasm --target wasm32-wasip2

# run the host: argv passes straight to the guest (no `run`, no `--`)
export RUST_LOG=info,opentelemetry_sdk=off
cargo run --example command-resolver -- greet Ada
cargo run --example command-resolver -- add 2 40
cargo run --example command-resolver -- fail 42; echo $?  # 42
```

The `--` above is cargo's own separator; the guest receives `greet Ada`
directly. Compare `examples/cli`, where the same guest runs through the
standard `run` grammar instead.
