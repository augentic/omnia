//! # WASI Messaging Service
//!
//! Serves `omnia:messaging` to guests over a [`MessagingClient`], and runs its
//! delivery trigger.

mod admin_impl;
mod consumer_impl;
mod default_impl;
mod log_impl;
mod producer_impl;
mod request_reply_impl;
mod resource;
mod server;
mod types_impl;

mod generated {
    #![allow(missing_docs, clippy::empty_enums, reason = "wasmtime bindgen output")]

    pub use self::omnia::messaging::types::Error;
    pub use crate::host::resource::ClientProxy;

    wasmtime::component::bindgen!({
        world: "messaging",
        path: "wit",
        imports: {
            default: store | tracing | trappable,
        },
        exports: {
            default: store | tracing | trappable,
        },
        with: {
            "omnia:messaging/types.client": ClientProxy,
        },
        trappable_error_type: {
            "omnia:messaging/types.error" => Error,
        },
    });
}

use std::fmt::Debug;
use std::sync::Arc;

pub use omnia_core::FutureResult;
use omnia_core::{Host, HostCtx, Provides, Runtime, StoreCtx};
use wasmtime::component::{Linker, ResourceTableError};

pub use self::default_impl::MessagingDefault;
use self::generated::omnia::messaging::{admin, consumer, log, producer, request_reply, types};
pub use self::generated::{Error, Messaging};
pub use self::resource::*;
pub use self::server::MessagingHandler;

/// Result type for messaging operations.
pub type Result<T> = std::result::Result<T, Error>;

/// Host-side service for `omnia:messaging`.
#[derive(Debug, HostCtx)]
#[host_ctx(client = MessagingClient)]
pub struct WasiMessaging;

impl<B: Provides<Self>> Host<B> for WasiMessaging {
    fn add_to_linker(linker: &mut Linker<StoreCtx<B>>) -> anyhow::Result<()> {
        types::add_to_linker::<_, Self>(linker, StoreCtx::view::<Self>)?;
        producer::add_to_linker::<_, Self>(linker, StoreCtx::view::<Self>)?;
        request_reply::add_to_linker::<_, Self>(linker, StoreCtx::view::<Self>)?;
        consumer::add_to_linker::<_, Self>(linker, StoreCtx::view::<Self>)?;
        log::add_to_linker::<_, Self>(linker, StoreCtx::view::<Self>)?;
        Ok(admin::add_to_linker::<_, Self>(linker, StoreCtx::view::<Self>)?)
    }

    async fn run(runtime: &Runtime<B>) -> anyhow::Result<()> {
        server::run(runtime).await
    }
}

/// Service contract for `omnia:messaging` backends.
///
/// This is implemented by the resource-specific provider of messaging
/// functionality. For example, a NATS, or a Kafka broker.
///
/// Message construction and mutation live on the host's [`Message`] struct;
/// backends only translate at the [`Endpoint`] seam.
pub trait MessagingClient: Debug + Send + Sync + 'static {
    /// Open a messaging endpoint handle.
    ///
    /// # Errors
    ///
    /// Returns an error if the connection fails.
    fn endpoint(&self) -> FutureResult<Arc<dyn Endpoint>>;
}

// a backend picks the kind by returning an `Error`; any other failure is
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
