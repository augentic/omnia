## Unreleased

### Changed

- Removed the `runtime!` macro's `program:` key: `mode: command` with a
  compiled-in deployment (`config:` or inline manifest keys) is now a
  direct command by default — raw argv passthrough with the reserved
  `--debug` / `--quiet` host log flags, no host `run` grammar. The program
  name (telemetry and guest `argv[0]`) defaults to the manifest name (first
  `[[guest]]` id). Command-mode binaries without a compiled-in deployment
  keep the `run` grammar
- Removed the `command_guest:` key and its plumbing
  (`DeploymentBuilder::command_guest`, `Runtime::with_command_guest`,
  `MainOptions::command_guest`): command mode routes to the sole static
  `wasi:cli/run` exporter, or to the guest entry marked `command = true`
  (macro `command: true`, `GuestEntry::command()`); at most one guest may
  carry the mark. The resolver-supplied command guest (fully dynamic
  deployment, empty guest set) is gone with the key — a direct command
  always compiles its manifest in

## 0.35.0

Released 2026-07-25

Paired production-backends release: [omnia-backends 0.29.x](https://github.com/augentic/omnia-backends/blob/main/RELEASES.md).

### Added

- Multi-guest registry: one process hosts many Wasm components on a shared
  engine/linker, selected by opaque `GuestId`, with instance-per-call
  instantiation, route tables, and host-mediated guest-to-guest linking
- Resolve-on-miss `GuestResolver` so missing guests can be faulted in at
  dispatch time (HTTP fallback, command routing, single-flight per identity)
- `runtime!` keys for resolver-backed deployments: `resolver:`,
  `command_guest:`, `program:`, and compile-time `config:` fallback
- Embedded guest bytes (`include_bytes!` / `Source::embedded`) for
  single-binary hosts alongside path-sourced guests
- Dynamic `Runtime::register` / `deregister` after bootstrap, including late
  import polyfill for guests admitted after static assembly
- Pooling allocator enabled by default, with environment-driven
  `RuntimeOptions` shared by `omnia compile` and `omnia create`
- `omnia-wasi-model` host interface and workspace packaging for model /
  working-tree workflows
- Stateless MCP stack in `omnia-guest` (JSON-RPC + Streamable HTTP) and
  testkit helpers for MCP grant recording
- Concurrent (`async func`) host-mediated link dispatch via
  `func_new_concurrent` / `send_concurrent`
- `omnia-testkit` (dev-only) and an integration-first testing posture

### Changed

- `omnia-sdk` renamed to `omnia-guest`; host/guest macro crates split into
  `omnia-host-macros` and `omnia-guest-macros`; `omnia-otel` folded into
  `omnia`; `omnia-wasi-jsondb` replaced by `omnia-wasi-docstore`
- Deployments center on `Manifest` / `omnia.toml` (`OMNIA_CONFIG`), with
  `[[mount]]` working-tree preopens and typed `DeploymentBuilder` paths
  (safe wasm vs trusted precompiled)
- Upgraded `wasmtime` to 47.0.2 (and matching `wasmtime-wasi*`),
  `wit-bindgen` to 0.60, `wasip3` to 0.7, and `cap-std` / `cap-fs-ext` to 4.x
- Compile-affecting runtime toggles now include `DEBUG_SYMBOLS` and
  `GENERATE_ADDRESS_MAP` so AOT compile and load stay aligned

<!-- Release notes generated using configuration in .github/release.yaml at main -->

## What's Changed
* Bump to 0.35.0 by @augentic-releases[bot] in https://github.com/augentic/omnia/pull/202
* Instrumentation fix by @andrewweston in https://github.com/augentic/omnia/pull/203
* Instance pooling by @andrewweston in https://github.com/augentic/omnia/pull/204
* Guest registry by @andrewweston in https://github.com/augentic/omnia/pull/205
* Implement wasi model by @andrewweston in https://github.com/augentic/omnia/pull/206
* Specify readiness testing by @andrewweston in https://github.com/augentic/omnia/pull/207
* MCP server for cursor-agent by @andrewweston in https://github.com/augentic/omnia/pull/208
* Post-upgrade testing and code review by @andrewweston in https://github.com/augentic/omnia/pull/209
* Async guest-2-guest linking by @andrewweston in https://github.com/augentic/omnia/pull/210
* style fenced code by @andrewweston in https://github.com/augentic/omnia/pull/211
* Specify-driven refactoring by @andrewweston in https://github.com/augentic/omnia/pull/212
* Streamline testing by @andrewweston in https://github.com/augentic/omnia/pull/213
* Replay by @andrewweston in https://github.com/augentic/omnia/pull/214
* MCP grants by @andrewweston in https://github.com/augentic/omnia/pull/215
* Runtime flexibility by @andrewweston in https://github.com/augentic/omnia/pull/216
* improve runtime config by @andrewweston in https://github.com/augentic/omnia/pull/217
* Dynamic guest resolver by @andrewweston in https://github.com/augentic/omnia/pull/218
* Dynamic guest resolver in runtime! by @andrewweston in https://github.com/augentic/omnia/pull/219
* Embed guest bytes by @andrewweston in https://github.com/augentic/omnia/pull/220
* Update to wasmtime 47.0.2 by @andrewweston in https://github.com/augentic/omnia/pull/221

**Full Changelog**: https://github.com/augentic/omnia/compare/v0.34.0...v0.35.0

---

Release notes for previous releases can be found on the respective release branches of the repository.

<!-- ARCHIVE_START -->
* [0.35.x](https://github.com/augentic/omnia/blob/release-0.35.0/RELEASES.md)
* [0.34.x](https://github.com/augentic/omnia/blob/release-0.34.0/RELEASES.md)
* [0.33.x](https://github.com/augentic/omnia/blob/release-0.33.0/RELEASES.md)
* [0.32.x](https://github.com/augentic/omnia/blob/release-0.32.0/RELEASES.md)

- [0.31.x](https://github.com/augentic/omnia/blob/release-0.31.0/RELEASES.md)
- [0.30.x](https://github.com/augentic/omnia/blob/release-0.30.0/RELEASES.md)
- [0.29.x](https://github.com/augentic/omnia/blob/release-0.29.0/RELEASES.md)
- [0.28.x](https://github.com/augentic/omnia/blob/release-0.28.0/RELEASES.md)
- [0.27.x](https://github.com/augentic/omnia/blob/release-0.27.0/RELEASES.md)
- [0.25.x](https://github.com/augentic/omnia/blob/release-0.25.0/RELEASES.md)
- [0.23.x](https://github.com/augentic/omnia/blob/release-0.23.0/RELEASES.md)
- [0.22.x](https://github.com/augentic/omnia/blob/release-0.22.0/RELEASES.md)
- [0.21.x](https://github.com/augentic/omnia/blob/release-0.21.0/RELEASES.md)
- [0.20.x](https://github.com/augentic/omnia/blob/release-0.20.0/RELEASES.md)
- [0.20.x](https://github.com/augentic/omnia/blob/release-0.20.0/RELEASES.md)
- [0.19.x](https://github.com/augentic/omnia/blob/release-0.19.0/RELEASES.md)
- [0.18.x](https://github.com/augentic/omnia/blob/release-0.18.0/RELEASES.md)
- [0.17.x](https://github.com/augentic/omnia/blob/release-0.17.0/RELEASES.md)
- [0.16.x](https://github.com/augentic/omnia/blob/release-0.16.0/RELEASES.md)
- [0.15.x](https://github.com/augentic/omnia/blob/release-0.15.0/RELEASES.md)
