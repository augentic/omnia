## 0.24.0

Unreleased

### Added

Added default implementations to guest capability traits. This means, in the most cases, guests
can be used without needing to implement the capability traits.

### Changed

Renamed this repo `qwasr` in anticipation of publishing to crates.io. All crates have been prefixed
with `qwasr-` to match the new name.

A number of bugs were fixed in the guest code, including:

- outgoing HTTP requests where the response body was not being read correctly
- resolved an issue with the `#[wasi_otel::instrument]` macro where guest traces and metrics were 
  not being exported.

<!-- Release notes generated using configuration in .github/release.yaml at main -->

**Full Changelog**: <https://github.com/augentic/qwasr/compare/v0.22.1...v0.24.0>

---

Release notes for previous releases can be found on the respective release
branches of the repository.

<!-- ARCHIVE_START -->
* [0.22.x](https://github.com/augentic/qwasr/blob/release-0.22.0/RELEASES.md)
* [0.21.x](https://github.com/augentic/qwasr/blob/release-0.21.0/RELEASES.md)
* [0.20.x](https://github.com/augentic/qwasr/blob/release-0.20.0/RELEASES.md)
* [0.20.x](https://github.com/augentic/qwasr/blob/release-0.20.0/RELEASES.md)
* [0.19.x](https://github.com/credibil/wrt/blob/release-0.19.0/RELEASES.md)
* [0.18.x](https://github.com/credibil/wrt/blob/release-0.18.0/RELEASES.md)
* [0.17.x](https://github.com/credibil/wrt/blob/release-0.17.0/RELEASES.md)
* [0.16.x](https://github.com/credibil/wrt/blob/release-0.16.0/RELEASES.md)
* [0.15.x](https://github.com/credibil/wrt/blob/release-0.15.0/RELEASES.md)
