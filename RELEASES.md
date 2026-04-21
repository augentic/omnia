## 0.31.0

Released 2026-04-21

### Changed

- Upgraded `wasmtime` to v44 -- adapted the incoming HTTP request handler to the
  new `Service::handle` API (no longer returns a `Task`); response body streaming
  now keeps `run_concurrent` alive until hyper finishes reading via a
  `BodyDoneWrapper` sentinel
- Restructured the outbound HTTP handler (`HttpDefault`) around wasmtime 44's
  `WasiHttpHooks` / `WasiHttpCtx` split. Outbound requests now reuse a shared
  `reqwest::Client` with connection pooling and a 10s connect timeout (configurable
  via `HTTP_CONNECT_TIMEOUT_SECS`) instead of building a new client per request.
  A one-off client is only built when a client certificate is required
- Removed `HOST` header deduplication in favour of letting `reqwest` set the
  header itself

<!-- Release notes generated using configuration in .github/release.yaml at main -->

## What's Changed

- Bump to 0.29.0 by @github-actions[bot] in https://github.com/augentic/omnia/pull/183
- Blobstore by @andrew-goldie in https://github.com/augentic/omnia/pull/185
- Small code quality improvements for blobstore by @andrew-goldie in https://github.com/augentic/omnia/pull/186
- Upgrade to wasmtime 43/44 by @karthik-phl in https://github.com/augentic/omnia/pull/188
- Removed HOST header altogether in favor of setting it by @karthik-phl in https://github.com/augentic/omnia/pull/189
- Upgrade to wasmtime 44 by @karthik-phl in https://github.com/augentic/omnia/pull/190
- Omnia 0.31.0 by @karthik-phl in https://github.com/augentic/omnia/pull/191

**Full Changelog**: https://github.com/augentic/omnia/compare/v0.29.0...v0.31.0

---

Release notes for previous releases can be found on the respective release
branches of the repository.

<!-- ARCHIVE_START -->

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
