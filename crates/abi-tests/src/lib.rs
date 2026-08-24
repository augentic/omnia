//! Guest–host seam tests for the Omnia workspace.
//!
//! This package carries no library code: the suite lives in `tests/`, one
//! integration-test target per scenario family, run process-per-test under
//! Nextest with the rest of the workspace. Build the guests it drives with
//! `cargo make test-guests` (`cargo make test` does both).
