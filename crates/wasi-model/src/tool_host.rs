use std::fmt::Debug;

pub use omnia_core::FutureResult;
use serde::Serialize;

use crate::Lend;

/// Host-side capabilities for one completion, lent to backends that need them.
pub trait ToolHost: Debug + Send + Sync {
    /// Run one of the request's tools through the completion's session.
    ///
    /// The guest's tool answers. The inner `Err` is the tool's own failure
    /// text, which the model reads and may repair from. The outer error is a
    /// hard host failure, which fails the completion:
    ///
    /// - an undeclared tool is `other`;
    /// - a spent tool-call budget, or no result within the timeout, is
    ///   `exhausted`;
    /// - an output or failure text over the size cap, or a session stream
    ///   the guest closed, is `invalid`.
    fn call_tool(&self, name: String, arguments: String) -> FutureResult<ToolOutcome>;

    /// Ask the guest whether `candidate` is the answer.
    ///
    /// `Ok(())` finishes the completion, and `Err(text)` is the correction
    /// turn the backend appends verbatim. It reaches the guest as a `check`
    /// call on the session's `calls` stream, outside the tool-call budget.
    /// The timeout and the size cap apply as they do to
    /// [`ToolHost::call_tool`], and so does its outer error.
    fn check(&self, candidate: String) -> FutureResult<Result<(), String>>;

    /// Bounded workspace read via the lent `wasi:filesystem` capability.
    fn read(&self, path: String) -> FutureResult<Vec<u8>>;

    /// Bounded workspace listing via the lent `wasi:filesystem` capability.
    fn list(&self, path: String) -> FutureResult<Vec<DirEntry>>;

    /// Accumulate an edit against the session's base tree.
    fn write(&self, path: String, bytes: Vec<u8>) -> FutureResult<()>;

    /// The workspace lent for this completion, when one was lent and resolved
    /// to an authorized mount.
    ///
    /// A backend whose processes work in it binds them to [`Lend::dir`],
    /// never to [`Lend::path`], and lets them write it only where
    /// [`Lend::writable`] holds.
    fn lend(&self) -> Option<&Lend> {
        None
    }
}

/// A tool's model-visible output or failure text.
pub type ToolOutcome = Result<String, String>;

/// One bounded directory entry returned by `ToolHost::list`; backends
/// serialise the listing for the model.
#[derive(Clone, Debug, Serialize)]
pub struct DirEntry {
    /// Entry name (never an OS path).
    pub name: String,
    /// Whether the entry is a directory.
    pub is_directory: bool,
}
