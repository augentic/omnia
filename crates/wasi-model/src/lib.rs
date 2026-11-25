#![doc = include_str!("../README.md")]

//! # WASI Model
//!
//! This module implements the runtime boundary for `omnia:model`: a guest
//! opens a model the deployment declares and calls `complete` to have a
//! prompt completed. It receives an answer it may have judged itself, without
//! ever seeing which backend produced it.

mod answer;
mod completion_impl;
mod default_impl;
mod request;
mod session;
mod tool_host;
mod workspace;

mod generated {
    #![allow(missing_docs, reason = "wasmtime bindgen output")]

    pub use omnia_wasi_types::Error;

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
            "omnia:types/types": omnia_wasi_types::types,
        },
        trappable_error_type: {
            "omnia:types/types.error" => Error,
        },
    });
}

use std::fmt::Debug;
use std::sync::Arc;

pub use omnia_core::FutureResult;
use omnia_core::{Host, HostCtx, HostView, Provides, StoreCtx};
use wasmtime::component::Linker;

pub use self::answer::{Answer, Exhausted, ToolTurn, Transcript, Usage};
pub use self::default_impl::{ConnectOptions, ModelDefault};
pub use self::generated::Error;
use self::generated::omnia::model::types::Host as TypesHost;
pub use self::generated::omnia::model::types::{
    Budget, Effort, Format, Grants, Location, Message, Role, Schema, WorkerGrant, WorkspaceGrant,
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
        omnia_wasi_types::types::add_to_linker::<_, Self>(linker, StoreCtx::view::<Self>)?;
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
    ///
    /// A backend with no bounds of its own returns [`Limits::default`]. A
    /// client that wraps another returns the wrapped client's.
    fn limits(&self) -> Limits;
}

impl TypesHost for HostView<'_, dyn ModelClient> {}
