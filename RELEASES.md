## 0.34.0

### Added

- `Error::Json` variant for returning domain-controlled JSON error responses
  with `application/json` content type
- `HttpError` now renders `Error::Json` variants as `application/json` responses
  instead of plain text
- ORM module consolidated into `omnia-sdk` (`omnia_sdk::orm`); the standalone
  `omnia-orm` crate has been removed

### Changed

- Request handler headers now use `HeaderMap<HeaderValue>` instead of
  `HeaderMap<String>`
- Upgraded `wasip3` from 0.5.0 to 0.6.0, resolving a guest deadlock under
  concurrent async calls
- Upgraded `wasmtime` from 45.0.0 to 45.0.2

### Fixed

- `wasi-messaging` message mutation methods (`set_content_type`, `set_payload`,
  `add_metadata`, `set_metadata`, `remove_metadata`) now update the resource
  in-place instead of pushing duplicates to the resource table

<!-- Release notes generated using configuration in .github/release.yaml at main -->

## What's Changed
* Bump to 0.33.0 by @augentic-releases[bot] in https://github.com/augentic/omnia/pull/197
* Updated supply chain for 0.33 release by @karthik-phl in https://github.com/augentic/omnia/pull/198
* Updated wasip3 to resolve guest deadlock by @karthik-phl in https://github.com/augentic/omnia/pull/199
* Bump to 0.34.0 by @augentic-releases[bot] in https://github.com/augentic/omnia/pull/200
* Sdk fixes by @karthik-phl in https://github.com/augentic/omnia/pull/201

**Full Changelog**: https://github.com/augentic/omnia/compare/v0.32.0...v0.34.0

---

Release notes for previous releases can be found on the respective release
branches of the repository.

<!-- ARCHIVE_START -->
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
