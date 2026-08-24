//! Host side of the `omnia:model/completion` boundary. Follows the shared
//! host-crate shape (see `wasi-keyvalue`), adding a per-completion [`ToolHost`]
//! that the `create` binding assembles from the store's mounts and the
//! session channels it mints for the completion.

mod answer;
mod default_impl;
mod gate;
mod model_impl;
mod session;
mod types;
mod workspace;

mod generated {
    #![allow(missing_docs)]

    pub use self::omnia::model::completion::Error;

    wasmtime::component::bindgen!({
        world: "model",
        path: "wit",
        imports: {
            default: store | tracing | trappable,
        },
        with: {
            "wasi:clocks": wasmtime_wasi::p3::bindings::clocks,
            "wasi:filesystem": wasmtime_wasi::p3::bindings::filesystem,
        },
        trappable_error_type: {
            "omnia:model/completion.error" => Error,
        },
    });
}

use std::fmt::Debug;
use std::sync::Arc;
use std::time::Duration;

pub use omnia::FutureResult;
use omnia::{HasMounts, Host, Server};
use wasmtime::component::{HasData, Linker};

pub use self::default_impl::ModelDefault;
pub use self::gate::validate as validate_request;
use self::generated::omnia::model::completion;
pub use self::generated::omnia::model::completion::{
    Effort, Error, Format, Function, Generation, Grants, Mcp, Message, Reply, Request, Role,
    Schema, Tool, WorkspaceGrant,
};
pub use self::types::{Answer, DirEntry, ToolTurn, Transcript, Usage};

/// Host-side service for `wasi-model` (a linked-only effect host).
#[derive(Debug)]
pub struct WasiModel;

impl HasData for WasiModel {
    type Data<'a> = WasiModelCtxView<'a>;
}

impl<T> Host<T> for WasiModel
where
    T: WasiModelView + HasMounts + 'static,
{
    fn add_to_linker(linker: &mut Linker<T>) -> anyhow::Result<()> {
        Ok(completion::add_to_linker::<_, Self>(linker, T::model)?)
    }
}

impl<B> Server<B> for WasiModel {}

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

/// The backend trait — the one place a provider's logic lives.
pub trait WasiModelCtx: Debug + Send + Sync + 'static {
    /// Produce an answer for the gate-validated `request`, optionally lending
    /// the per-completion [`ToolHost`] to backends that drive an in-process
    /// tool loop. The host has already taken the lent `grants.workspace`
    /// borrow, so it is always `None` here.
    fn complete(&self, request: Request, tool_host: Arc<dyn ToolHost>) -> FutureResult<Answer>;

    /// Session bounds the host enforces for this backend's completions;
    /// override to tighten (test probes shrink them).
    fn limits(&self) -> SessionLimits {
        SessionLimits::default()
    }
}

/// Forward the backend trait.
impl WasiModelCtx for Box<dyn WasiModelCtx> {
    fn complete(&self, request: Request, tool_host: Arc<dyn ToolHost>) -> FutureResult<Answer> {
        (**self).complete(request, tool_host)
    }

    fn limits(&self) -> SessionLimits {
        (**self).limits()
    }
}

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

// An untyped host failure is a `backend` error at the boundary.
omnia::host_error!(Error, Backend);
omnia::wasi_view!(Model);
