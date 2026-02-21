# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/),
and this project adheres to [Semantic Versioning](https://semver.org/).

## [Unreleased]

## [0.27.0] - TBD

### Changed

- Rebranded from Credibil to Augentic
- Prepared all crates for crates.io publishing
- Switched from Rust edition 2024 (nightly) to edition 2021 (stable)
- Replaced `unimplemented!()` and `todo!()` panics with proper error handling
- Escalated `missing_docs` lint from warn to deny
- Added `#![forbid(unsafe_code)]` to all crates except `omnia` (which requires unsafe for wasmtime interop)
- Improved crate descriptions for crates.io display
- Removed private registry (`credibil`) references from workspace dependencies

---

Release notes for previous releases can be found on the respective release
branches of the repository.

- [0.25.x](https://github.com/augentic/omnia/blob/release-0.25.0/RELEASES.md)
- [0.23.x](https://github.com/augentic/omnia/blob/release-0.23.0/RELEASES.md)
- [0.22.x](https://github.com/augentic/omnia/blob/release-0.22.0/RELEASES.md)
- [0.21.x](https://github.com/augentic/omnia/blob/release-0.21.0/RELEASES.md)
- [0.20.x](https://github.com/augentic/omnia/blob/release-0.20.0/RELEASES.md)
- [0.19.x](https://github.com/augentic/omnia/blob/release-0.19.0/RELEASES.md)
- [0.18.x](https://github.com/augentic/omnia/blob/release-0.18.0/RELEASES.md)
- [0.17.x](https://github.com/augentic/omnia/blob/release-0.17.0/RELEASES.md)
- [0.16.x](https://github.com/augentic/omnia/blob/release-0.16.0/RELEASES.md)
- [0.15.x](https://github.com/augentic/omnia/blob/release-0.15.0/RELEASES.md)
