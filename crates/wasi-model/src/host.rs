//! Host side of the `omnia:model/completion` boundary. Follows the shared
//! host-crate shape (see `wasi-keyvalue`), adding a per-completion [`ToolHost`]
//! that the `create` binding assembles from the store's mounts and the
//! session channels it mints for the completion.

mod answer;
mod default_impl;
mod gate;
mod model_impl;
mod resource;
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
pub use self::types::{Answer, ToolTurn, Transcript, Usage};
pub use self::resource::*;

/// Host-side service for `wasi-model`.
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



/// The backend trait — the one place a provider's logic lives.
pub trait WasiModelCtx: Debug + Send + Sync + 'static {
    /// Call model backend with a prose to evaluate. 
    /// [`ToolHost`] provides closure-like  support for an in-process tool
    /// loop — the backend tool loop can use it to request more information
    /// from the host.
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



// An untyped host failure is a `backend` error at the boundary.
omnia::host_error!(Error, Backend);
omnia::wasi_view!(Model);
