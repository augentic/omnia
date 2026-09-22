## 0.37.0

Unreleased

### Added

- One tracing level for the whole process, selected by verbosity flags. A
  run's level is the flag, else the process `RUST_LOG`, else the mode's
  default — `info` for a command, `warn` for a server (`Mode::level`) — and
  it governs the host console and every guest alike: each guest's WASI
  environment carries it as `RUST_LOG` (a selected level replaces the
  process variable there; a bare run fills it only when unset), and the
  process environment is never written. Each `-v`/`--verbose` steps the
  level one rung up the scale `off`, `error`, `warn`, `info`, `debug`,
  `trace` from the mode's default and each `-q`/`--quiet` one rung down,
  clamped at the ends, so a command reads `-q` warn, `-qq` error, `-v`
  debug, `-vv` trace, and a server `-q` error, `-qq` off, `-v` info, `-vv`
  debug, `-vvv` trace. On the `run` grammar the flags are global to the
  command (`bin -v run …`, `bin run -v …`; `-v` beside `-q` is a usage
  error); on the direct-command path the host reads them out of argv before
  `--` and forwards argv verbatim, so a command guest declares the same flags
  by flattening `omnia_sdk::api::command::Verbosity` into its grammar, which
  lists them in help and completions, accepts them, and refuses the pair with
  its own usage error — the guest never acts on the values. Embedders select
  the level in code with `DeploymentBuilder::level(LevelFilter)` (`omnia`
  re-exports `LevelFilter`), `Telemetry::fallback(level)` sets the console's
  fallback for an unset `RUST_LOG` (`WARN` when not called, as before), and
  the test host's `Deployment::level` scripts a guest's `RUST_LOG` without
  touching the suite's environment. A bare command run's host console now
  opens at `info`, where it opened at `warn`.
- The dispatch-chain context lives on the guest store. Every store is built
  at a `ChainCtx` (`StoreConfig::chain`, `StoreBase::chain`):
  `Runtime::store()` builds a server root, `Runtime::store_in(chain)` any
  other, and a `StoreFactory` takes the context its callee runs at, so the
  command driver, the trigger hosts, and `call_fresh` carry no ambient
  scope. The link relay reads the calling guest's context from its store
  through `HasChain` (implemented by `StoreCtx` beside `HasMounts` and
  `HasExtensions`), `ChainPolicy::enter(&caller, &target)` derives the
  callee's, and `Dispatcher::invoke` takes the caller's context ahead of
  the target.

### Changed

- `ChainCtx` is no longer `Default`: a root is `ChainCtx::server()` or
  `ChainCtx::command()`. `as_command_chain(fut)` is now a store built at the
  command root, `runtime.build_store(runtime.store_in(ChainCtx::command()))`,
  and `Dispatcher::invoke` takes the caller's `ChainCtx` before the target.

---

Release notes for previous releases can be found on the respective release branches of the repository.

<!-- ARCHIVE_START -->
* [0.36.x](https://github.com/augentic/omnia/blob/release-0.36.0/RELEASES.md)
* [0.35.x](https://github.com/augentic/omnia/blob/release-0.35.0/RELEASES.md)
* [0.34.x](https://github.com/augentic/omnia/blob/release-0.34.0/RELEASES.md)
* [0.33.x](https://github.com/augentic/omnia/blob/release-0.33.0/RELEASES.md)
* [0.32.x](https://github.com/augentic/omnia/blob/release-0.32.0/RELEASES.md)
* [0.31.x](https://github.com/augentic/omnia/blob/release-0.31.0/RELEASES.md)
* [0.30.x](https://github.com/augentic/omnia/blob/release-0.30.0/RELEASES.md)
* [0.29.x](https://github.com/augentic/omnia/blob/release-0.29.0/RELEASES.md)
* [0.28.x](https://github.com/augentic/omnia/blob/release-0.28.0/RELEASES.md)
* [0.27.x](https://github.com/augentic/omnia/blob/release-0.27.0/RELEASES.md)
* [0.25.x](https://github.com/augentic/omnia/blob/release-0.25.0/RELEASES.md)
* [0.23.x](https://github.com/augentic/omnia/blob/release-0.23.0/RELEASES.md)
* [0.22.x](https://github.com/augentic/omnia/blob/release-0.22.0/RELEASES.md)
* [0.21.x](https://github.com/augentic/omnia/blob/release-0.21.0/RELEASES.md)
* [0.20.x](https://github.com/augentic/omnia/blob/release-0.20.0/RELEASES.md)
* [0.19.x](https://github.com/augentic/omnia/blob/release-0.19.0/RELEASES.md)
* [0.18.x](https://github.com/augentic/omnia/blob/release-0.18.0/RELEASES.md)
* [0.17.x](https://github.com/augentic/omnia/blob/release-0.17.0/RELEASES.md)
* [0.16.x](https://github.com/augentic/omnia/blob/release-0.16.0/RELEASES.md)
* [0.15.x](https://github.com/augentic/omnia/blob/release-0.15.0/RELEASES.md)
