## 0.33.0

Unreleased

### Changed

- **wasip3** upgraded from 0.5.0 to 0.6.0 — resolves a `wit-bindgen` mismatch between `wasi-messaging` and `wasi-http` that caused a guest deadlock when making outbound HTTP calls from a messaging handler. The mismatch meant the `body_writer` spawned by wasip3 landed in a different `SPAWNED` queue than the active executor.
- **wasmtime** and related crates (`wasmtime-wasi`, `wasmtime-wasi-config`, `wasmtime-wasi-http`, `wasmtime-wasi-io`) upgraded from 45.0.0 to 45.0.1.
- Other minor dependency updates.
- The messaging example now exercises outbound HTTP from within a messaging handler, validating the deadlock fix end-to-end.

<!-- Release notes generated using configuration in .github/release.yaml at main -->

## What's Changed

- Bump to 0.31.0 by @github-actions[bot] in https://github.com/augentic/omnia/pull/192
- Circuit breaker and retry support in wasi-http by @karthik-phl in https://github.com/augentic/omnia/pull/193
- Upgraded to Rust 1.95 and updated rusqlite to fix audit issues by @karthik-phl in https://github.com/augentic/omnia/pull/194
- Minor orm deduplications by @andrewweston in https://github.com/augentic/omnia/pull/195
- Remove wasi-http resiliency constructs by @karthik-phl in https://github.com/augentic/omnia/pull/196
- Bump to 0.33.0 by @augentic-releases[bot] in https://github.com/augentic/omnia/pull/197
- Updated supply chain for 0.33 release by @karthik-phl in https://github.com/augentic/omnia/pull/198
- Updated wasip3 to resolve guest deadlock by @karthik-phl in https://github.com/augentic/omnia/pull/199

## New Contributors

- @augentic-releases[bot] made their first contribution in https://github.com/augentic/omnia/pull/197

**Full Changelog**: https://github.com/augentic/omnia/compare/v0.31.0...v0.33.0

---

Release notes for previous releases can be found on the respective release
branches of the repository.

<!-- ARCHIVE_START -->

- [0.33.x](https://github.com/augentic/omnia/blob/release-0.33.0/RELEASES.md)
- [0.32.x](https://github.com/augentic/omnia/blob/release-0.32.0/RELEASES.md)

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
* [0.20.x](https://github.com/augentic/omnia/blob/release-0.20.0/RELEASES.md)
* [0.19.x](https://github.com/augentic/omnia/blob/release-0.19.0/RELEASES.md)
* [0.18.x](https://github.com/augentic/omnia/blob/release-0.18.0/RELEASES.md)
* [0.17.x](https://github.com/augentic/omnia/blob/release-0.17.0/RELEASES.md)
* [0.16.x](https://github.com/augentic/omnia/blob/release-0.16.0/RELEASES.md)
* [0.15.x](https://github.com/augentic/omnia/blob/release-0.15.0/RELEASES.md)
