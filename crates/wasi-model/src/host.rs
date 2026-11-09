//! # WASI Model Service
//!
//! Host side of the `omnia:model/completion` boundary. Follows the shared
//! host-crate shape (see `wasi-keyvalue`), adding a per-completion [`ToolHost`]
//! that the `create` binding assembles from the store's mounts and the
//! session channels it mints for the completion.

mod completion_impl;
mod default_impl;
mod session;
mod workspace;

mod generated {
    #![allow(missing_docs, reason = "wasmtime bindgen output")]

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

pub use default_impl::ModelDefault;
pub use omnia_backend::model::{
    Answer, DirEntry, Effort, Error, Format, Function, Generation, Limits, Mcp, McpGrant, Message,
    ModelClient, Reply, Request, Role, Schema, SchemaFormat, Tool, ToolCall, ToolHost, ToolOutcome,
    ToolTurn, Transcript, Usage, WorkerGrant,
};
pub use omnia_core::FutureResult;
use omnia_core::{HasMounts, Host, HostCtx, Server, StoreView};
use wasmtime::component::Linker;

use self::generated::omnia::model::completion;
pub use self::generated::omnia::model::completion::{Error as WitError, Grants, WorkspaceGrant};

/// Result type for model operations.
pub type Result<T> = std::result::Result<T, WitError>;

/// Host-side service for `wasi:model`.
#[derive(Debug, HostCtx)]
pub struct WasiModel;

impl<T> Host<T> for WasiModel
where
    T: StoreView<Self> + HasMounts + 'static,
{
    fn add_to_linker(linker: &mut Linker<T>) -> anyhow::Result<()> {
        Ok(completion::add_to_linker::<_, Self>(linker, T::view)?)
    }
}

impl<B> Server<B> for WasiModel {}

// An untyped host failure is a `backend` error at the boundary.
omnia_core::host_error!(WitError, Backend);
