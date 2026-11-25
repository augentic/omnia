# omnia-cli

The command-line grammar for [omnia](https://github.com/augentic/omnia) runtime binaries. `[-v…|-q…] run [wasm] [--manifest <omnia.toml>] [--mount …] [--env …] -- args…` runs a deployment, with the manifest resolved by the `--manifest` › `OMNIA_MANIFEST` › positional wasm › compiled-in ladder and the process tracing level stepped from the mode's default by each `-v`/`--verbose` (up) and `-q`/`--quiet` (down), global flags that parse before or after `run`. `compile <wasm> [-o <path>] [-t <triple>]` pre-compiles a component. The grammar decides over paths and strings and returns a `Plan`. `omnia` materializes a `RunPlan` into a `Manifest` / `DeploymentBuilder` and drives the runtime, and compiles a `CompilePlan` under the compile-affecting settings its environment sets. A direct-command binary never parses this grammar: `omnia` forwards argv to the command guest verbatim.

Depend on the `omnia` facade, not on this crate: the `runtime!` macro's generated `main` reaches this crate through `omnia::main`. A direct dependency on `omnia-cli` (or on `omnia-core`) is never needed by a deployment.

## License

MIT OR Apache-2.0
