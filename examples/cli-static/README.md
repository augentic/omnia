# Direct Command Example

Demonstrates a compiled-in command deployment: a guest embedded from the
`runtime!` invocation's inline `guests:` list. The examples package's
`build.rs` compiles the guest for `wasm32-wasip2` and names the artifact in
`CLI_WASM`; the macro embeds those bytes with `include_bytes!` and names the
guest by the file's stem, `cli_wasm`. The guest is the sole `wasi:cli/run`
exporter, so command mode routes to it with no configuration.

Because the deployment is compiled in, command mode makes the binary a
**direct command** with no host CLI: there is no `run` subcommand and no
`--manifest`/`OMNIA_MANIFEST`/positional-wasm override — every argument is
forwarded to the guest verbatim.

## Quick Start

```bash
# run the host: argv passes straight to the guest (no `run`, no `--`);
# the guest builds with the host
cargo run --example cli-static -- greet Ada
cargo run --example cli-static -- add 2 40
cargo run --example cli-static -- fail not-found; echo $?  # 2
```

The `--` above is cargo's own separator; the guest receives `greet Ada`
directly. Compare `examples/cli`, where the same guest runs through the
standard `run` grammar instead, read from disk at start.
