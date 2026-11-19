//! Behaviour rows every backend of an `omnia:*` interface meets.
//!
//! A row is one invariant of an interface, written once as an `async fn`
//! that panics when the backend behind the host breaks it. A guest component
//! runs the rows over the [`omnia_sdk`] bindings, against a real host and its
//! backend, so the rows run in guests only.
//!
//! The rows are the testable half of the
//! [provider obligations](https://github.com/augentic/omnia/blob/main/docs/reference/provider-contract.md#obligations).

pub mod blob;
pub mod document;
pub mod sql;
pub mod state;

use std::fmt::Debug;

use omnia_sdk::Error;

#[track_caller]
fn expect_code<T: Debug, E: Into<Error>>(result: Result<T, E>, code: &str, call: &str) {
    match result.map_err(Into::into) {
        Ok(value) => panic!("{call} returned {value:?}, not a `{code}` failure"),
        Err(err) => assert!(is_code(&err, code), "{call} failed with `{err}`, not `{code}`"),
    }
}

// a binding failure the rows expect is a `ServerError`, so a backend
// answering with another variant breaks the row even under the right code
fn is_code(err: &Error, code: &str) -> bool {
    matches!(err, Error::ServerError { code: found, .. } if found == code)
}
