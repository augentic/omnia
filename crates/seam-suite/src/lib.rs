//! Consolidated guest–host seam tests for the Omnia workspace.
//!
//! This package carries no library code: the suite lives in `tests/seam/`,
//! one integration-test binary whose scenarios share a process (and therefore
//! the runtime fixtures). Build the guests it drives with
//! `cargo make build-test-guests`, then run `cargo make test-seam`.
