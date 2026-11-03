//! Service contract for `omnia:model` backends.

use std::fmt::Debug;
use std::sync::Arc;

use super::{Answer, Limits, Request, ToolHost};
use crate::FutureResult;

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
