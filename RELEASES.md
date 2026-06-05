## 0.32.0

Released 2026-06-05

### Changed

- Upgraded the wasmtime stack (`wasmtime`, `wasmtime-wasi`, `wasmtime-wasi-config`,
  `wasmtime-wasi-http`, `wasmtime-wasi-io`) from v44 to v45.0.0
- Upgraded MSRV from Rust 1.93 to 1.95
- Upgraded OpenTelemetry to 0.32 (`opentelemetry`, `opentelemetry_sdk`) and
  `tracing-opentelemetry` to 0.33
- Upgraded `rusqlite` from 0.39 to 0.40 (and `libsqlite3-sys` from 0.37 to
  0.38) to address `cargo audit` findings; bumped `rkyv` from 0.8.15 to 0.8.16
- WASM32 gating has been relaxed for the ORM crate and some refactoring done to
  reduce boilerplate

<!-- Release notes generated using configuration in .github/release.yaml at main -->

## What's Changed
* Upgraded to Rust 1.95 and updated rusqlite to fix audit issues by @karthik-phl in https://github.com/augentic/omnia/pull/194
* Minor orm deduplications by @andrewweston in https://github.com/augentic/omnia/pull/195

**Full Changelog**: https://github.com/augentic/omnia/compare/v0.31.0...v0.32.0

---

Release notes for previous releases can be found on the respective release
branches of the repository.

<!-- ARCHIVE_START -->
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
