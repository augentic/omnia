
use std::time::Duration;

pub use omnia::FutureResult;

use crate::host::types::DirEntry;

/// Host-side capabilities for one completion, lent to backends that need them.
pub trait ToolHost: Send + Sync {
    /// Run one declared function tool through the completion's session: the
    /// guest's tool closure answers. The outer error is a hard host failure
    /// (undeclared tool, exhausted budget, closed session, oversize result,
    /// timeout); the inner `Err` is the tool's own model-visible failure
    /// text, fed back to the model as repairable content.
    fn call_tool(&self, name: String, arguments: String) -> FutureResult<Result<String, String>>;

    /// Bounded workspace read via the lent `wasi:filesystem` capability.
    fn read(&self, path: String) -> FutureResult<Vec<u8>>;

    /// Bounded workspace listing via the lent `wasi:filesystem` capability.
    fn list(&self, path: String) -> FutureResult<Vec<DirEntry>>;

    /// Accumulate an edit against the session's base tree.
    fn write(&self, path: String, bytes: Vec<u8>) -> FutureResult<()>;

    /// The absolute host path of the lent workspace, when one was lent for
    /// this completion and resolved to an authorized mount.
    fn local_path(&self) -> Option<&std::path::Path> {
        None
    }
}

/// Session bounds the host enforces per completion, in `wasi-model`,
/// regardless of backend.
#[derive(Clone, Copy, Debug)]
pub struct SessionLimits {
    /// Tool calls one completion may issue before `budget-exhausted`.
    pub max_tool_calls: u32,
    /// Byte cap on a single tool result's output.
    pub max_result_bytes: usize,
    /// How long the host waits for the guest to answer one tool call.
    pub tool_timeout: Duration,
}

impl Default for SessionLimits {
    fn default() -> Self {
        Self {
            max_tool_calls: 32,
            max_result_bytes: 1 << 20,
            tool_timeout: Duration::from_secs(60),
        }
    }
}