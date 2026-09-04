# omnia-cli

The `run` command-line grammar for [omnia](https://github.com/augentic/omnia) runtime binaries: `run [wasm] [--config <omnia.toml>] [--mount …] [--plugins …] -- args…`, with the deployment manifest resolved by the `--config` › `OMNIA_CONFIG` › positional wasm › compiled-in ladder. The `omnia` facade's `cli` feature selects this crate; it decides the `run` source over paths and strings, materializes that decision into a deployment, and hands every direct command, untouched, down to the `omnia-core` runtime spine.

Depend on the `omnia` facade, not on this crate: `omnia::Cli`, `omnia::Command`, and `omnia::Parser` re-export from here behind the `cli` feature, and the `runtime!` macro's generated `main` reaches this crate through `omnia::main`. A direct dependency on `omnia-cli` (or on `omnia-core`) is never needed by a deployment.

## License

MIT OR Apache-2.0
