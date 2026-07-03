//! # WASI WebSocket Service
//!
//! This module implements a runtime server for websocket

mod client_impl;
mod default_impl;
mod resource;
mod server;
mod types_impl;

mod generated {
    #![allow(missing_docs)]

    pub use self::omnia::websocket::types::Error;
    pub use crate::host::resource::{ClientProxy, EventProxy};

    wasmtime::component::bindgen!({
        world: "duplex",
        path: "wit",
        imports: {
            default: store | tracing | trappable,
        },
        exports: {
            default: store | tracing | trappable,
        },
        with: {
            "omnia:websocket/types.client": ClientProxy,
            "omnia:websocket/types.event": EventProxy,
        },
        trappable_error_type: {
            "omnia:websocket/types.error" => Error,
        },
    });
}

use std::fmt::Debug;
use std::sync::Arc;

pub use omnia::FutureResult;
use omnia::{Host, Runtime, Server, StoreCtx};
use wasmtime::component::{HasData, Linker, ResourceTableError};

pub use self::default_impl::WebSocketDefault;
pub use self::generated::Duplex;
pub use self::generated::omnia::websocket::types::Error;
use self::generated::omnia::websocket::{client, types as generated_types};
pub use self::resource::*;

/// Result type for WebSocket operations.
pub type Result<T> = anyhow::Result<T, Error>;

/// Host-side service for `wasi:websocket`.
#[derive(Clone, Debug)]
pub struct WasiWebSocket;

impl HasData for WasiWebSocket {
    type Data<'a> = WasiWebSocketCtxView<'a>;
}

impl<T> Host<T> for WasiWebSocket
where
    T: WasiWebSocketView + 'static,
{
    fn add_to_linker(linker: &mut Linker<T>) -> anyhow::Result<()> {
        client::add_to_linker::<_, Self>(linker, T::websocket)?;
        Ok(generated_types::add_to_linker::<_, Self>(linker, T::websocket)?)
    }
}

impl<B> Server<B> for WasiWebSocket
where
    B: Clone + Send + Sync + 'static,
    StoreCtx<B>: WasiWebSocketView,
{
    const IS_SERVER: bool = true;

    async fn run(&self, state: &Runtime<B>) -> anyhow::Result<()> {
        server::run(state).await
    }
}

/// A trait which provides internal WASI WebSocket context.
///
/// This is implemented by the resource-specific provider of WebSocket
/// functionality.
pub trait WasiWebSocketCtx: Debug + Send + Sync + 'static {
    /// Connect to the WebSocket service and return a socket.
    ///
    /// # Errors
    ///
    /// Returns an error if the connection fails.
    fn connect(&self) -> FutureResult<Arc<dyn Client>>;

    /// Create a new event with the given payload.
    ///
    /// # Errors
    ///
    /// Returns an error if event creation fails.
    fn new_event(&self, data: Vec<u8>) -> anyhow::Result<Arc<dyn Event>>;
}

impl From<anyhow::Error> for Error {
    fn from(err: anyhow::Error) -> Self {
        Self::Other(err.to_string())
    }
}

impl From<ResourceTableError> for Error {
    fn from(err: ResourceTableError) -> Self {
        Self::Other(err.to_string())
    }
}

impl From<wasmtime::Error> for Error {
    fn from(err: wasmtime::Error) -> Self {
        Self::Other(err.to_string())
    }
}

omnia::wasi_view!(WebSocket);
