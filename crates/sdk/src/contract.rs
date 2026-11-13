//! Behaviour rows every provider of a capability meets.
//!
//! A row is one invariant of a capability trait, written once as a generic
//! `async fn` that panics when the provider breaks it. A provider's suite
//! runs every row of the traits it implements:
//!
//! - `omnia-test` runs them natively over its stateful doubles.
//! - A guest runs them over [`WasiProvider`](crate::WasiProvider), against a
//!   real host and its backend.
//!
//! The rows are the testable half of the
//! [provider obligations](https://github.com/augentic/omnia/blob/main/docs/reference/provider-contract.md#obligations).

pub mod document;
pub mod sql;
pub mod state;

use std::fmt::Debug;

use crate::capability::{ErrorKind, Result};

#[track_caller]
fn expect_kind<T: Debug>(result: Result<T>, kind: ErrorKind, call: &str) {
    match result {
        Ok(value) => panic!("{call} returned {value:?}, not a `{kind}` failure"),
        Err(err) => assert_eq!(err.kind(), kind, "{call} failed with `{err}`"),
    }
}
