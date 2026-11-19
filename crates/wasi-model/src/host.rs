//! # WASI Model Service
//!
//! Host side of `omnia:model`. Follows the shared host-crate shape (see
//! `wasi-keyvalue`), adding a per-completion [`ToolHost`] that a `complete`
//! binding assembles from the store's mounts and the session channels it
//! mints for the completion.

mod answer;
mod completion_impl;
mod default_impl;
mod request;
mod session;
mod tool_host;
mod workspace;

mod generated {
    #![allow(missing_docs, reason = "wasmtime bindgen output")]

    pub use self::omnia::model::types::Error;
    pub use super::completion_impl::ModelHandle;

    wasmtime::component::bindgen!({
        world: "imports",
        path: "wit",
        imports: {
            default: store | tracing | trappable,
        },
        with: {
            "wasi:clocks": wasmtime_wasi::p3::bindings::clocks,
            "wasi:filesystem": wasmtime_wasi::p3::bindings::filesystem,
            "omnia:model/completion.model": ModelHandle,
        },
        trappable_error_type: {
            "omnia:model/types.error" => Error,
        },
    });
}

use std::fmt::Debug;
use std::sync::Arc;

pub use omnia_core::FutureResult;
use omnia_core::{Host, HostCtx, HostView, Provides, StoreCtx};
use wasmtime::component::{Linker, ResourceTableError};

pub use self::answer::{Answer, Exhausted, ToolTurn, Transcript, Usage};
pub use self::default_impl::{ConnectOptions, ModelDefault};
pub use self::generated::Error;
use self::generated::omnia::model::types::Host as TypesHost;
pub use self::generated::omnia::model::types::{
    Format, Grants, Location, Message, Role, Schema, WorkerGrant,
};
use self::generated::omnia::model::{completion, types};
pub use self::request::{Function, Generation, Request, Tool};
pub use self::session::Limits;
pub use self::tool_host::*;

/// Result type for model operations.
pub type Result<T> = std::result::Result<T, Error>;

/// Host-side service for `omnia:model`.
#[derive(Debug, HostCtx)]
#[host_ctx(client = ModelClient)]
pub struct WasiModel;

impl<B: Provides<Self>> Host<B> for WasiModel {
    fn add_to_linker(linker: &mut Linker<StoreCtx<B>>) -> anyhow::Result<()> {
        types::add_to_linker::<_, Self>(linker, StoreCtx::view::<Self>)?;
        Ok(completion::add_to_linker::<_, Self>(linker, StoreCtx::view::<Self>)?)
    }
}

/// Service contract for `omnia:model` backends.
///
/// This is implemented by the resource-specific provider of model
/// functionality. For example, the echo default, or a `genai`-backed
/// provider.
pub trait ModelClient: Debug + Send + Sync + 'static {
    /// Run the model on `request` until it has an answer.
    ///
    /// [`Request::model`] is the name the guest opened the model by. The SDK
    /// opens `default` for a request that names none. The model runs the
    /// guest's tools and checks through `tool_host`.
    ///
    /// A failure reaches the guest as follows:
    ///
    /// - An [`Error`] keeps its case, so a model the deployment does not
    ///   declare is [`Error::NotFound`].
    /// - An [`Exhausted`] is a budget that ran out before the model had an
    ///   answer.
    /// - Any other failure is [`Error::Other`], with its full context chain.
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

// a backend picks the case by returning an `Error`; any other failure is
// `other`, with its full context chain
impl From<anyhow::Error> for Error {
    fn from(err: anyhow::Error) -> Self {
        err.downcast::<Self>().unwrap_or_else(|err| Self::Other(format!("{err:#}")))
    }
}

impl From<wasmtime::Error> for Error {
    fn from(err: wasmtime::Error) -> Self {
        Self::Other(format!("{err:#}"))
    }
}

impl From<ResourceTableError> for Error {
    fn from(err: ResourceTableError) -> Self {
        Self::Other(err.to_string())
    }
}

impl TypesHost for HostView<'_, dyn ModelClient> {
    fn convert_error(&mut self, err: Error) -> wasmtime::Result<Error> {
        Ok(err)
    }
}
