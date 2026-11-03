//! Service contract for `omnia:model` backends.

use std::fmt::Debug;
use std::sync::Arc;

use super::{Answer, DirEntry, Limits, Request, ToolOutcome};
use crate::FutureResult;

/// Host-side capabilities for one completion, lent to backends that need them.
pub trait ToolHost: Debug + Send + Sync {
    /// Run one declared function tool through the completion's session.
    fn call_tool(&self, name: String, arguments: String) -> FutureResult<ToolOutcome>;

    /// Ask the guest whether `candidate` is the answer.
    fn check(&self, candidate: String) -> FutureResult<Result<(), String>>;

    /// Bounded workspace read via the lent capability.
    fn read(&self, path: String) -> FutureResult<Vec<u8>>;

    /// Bounded workspace listing via the lent capability.
    fn list(&self, path: String) -> FutureResult<Vec<DirEntry>>;

    /// Accumulate an edit against the session's base tree.
    fn write(&self, path: String, bytes: Vec<u8>) -> FutureResult<()>;

    /// The absolute host path of the lent workspace, when one was lent for
    /// this completion and resolved to an authorized mount.
    fn local_path(&self) -> Option<&std::path::Path> {
        None
    }
}

/// Service contract for `omnia:model` backends.
pub trait ModelClient: Debug + Send + Sync + 'static {
    /// Evaluate `request` with `tool_host` for in-process tool and workspace calls.
    ///
    /// When a workspace was lent, `request.workspace` is the matched mount-relative
    /// subpath and [`ToolHost::local_path`] is the resolved host path.
    fn complete(&self, request: Request, tool_host: Arc<dyn ToolHost>) -> FutureResult<Answer>;

    /// Session bounds the host enforces for this backend's completions.
    fn limits(&self) -> Limits {
        Limits::default()
    }
}

impl ModelClient for Box<dyn ModelClient> {
    fn complete(&self, request: Request, tool_host: Arc<dyn ToolHost>) -> FutureResult<Answer> {
        (**self).complete(request, tool_host)
    }

    fn limits(&self) -> Limits {
        (**self).limits()
    }
}
