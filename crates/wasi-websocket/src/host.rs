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

    pub use wasi::websocket::types::Error;

    pub use crate::host::resource::{ClientProxy, EventProxy};

    wasmtime::component::bindgen!({
        world: "websocket",
        path: "wit",
        imports: {
            default: store | tracing | trappable,
        },
        exports: {
            default: store | tracing | trappable,
        },
        with: {
            "wasi:websocket/types.client": ClientProxy,
            "wasi:websocket/types.event": EventProxy,
        },
        trappable_error_type: {
            "wasi:websocket/types.error" => Error,
        },
    });
}

use std::fmt::Debug;
use std::sync::Arc;

pub use qwasr::FutureResult;
use qwasr::{Host, Server, State};
use wasmtime::component::{HasData, Linker};
use wasmtime_wasi::{ResourceTable, ResourceTableError};

pub use self::default_impl::WebSocketDefault;
pub use self::generated::Websocket;
pub use self::generated::wasi::websocket::types::Error;
use self::generated::wasi::websocket::{client, types as generated_types};
pub use self::resource::*;

/// Result type for WebSocket operations.
pub type Result<T, E = Error> = anyhow::Result<T, E>;

/// Host-side service for `wasi:websocket`.
#[derive(Clone, Debug)]
pub struct WasiWebSocket;

impl HasData for WasiWebSocket {
    type Data<'a> = WasiWebSocketCtxView<'a>;
}

impl<T> Host<T> for WasiWebSocket
where
    T: WebSocketView + 'static,
{
    fn add_to_linker(linker: &mut Linker<T>) -> anyhow::Result<()> {
        client::add_to_linker::<_, Self>(linker, T::websocket)?;
        Ok(generated_types::add_to_linker::<_, Self>(linker, T::websocket)?)
    }
}

impl<S> Server<S> for WasiWebSocket
where
    S: State,
    S::StoreCtx: WebSocketView,
{
    async fn run(&self, state: &S) -> anyhow::Result<()> {
        server::run(state).await
    }
}

/// A trait which provides internal WASI WebSocket state.
///
/// This is implemented by the `T` in `Linker<T>` — a single type shared across
/// all WASI components for the runtime build.
pub trait WebSocketView: Send {
    /// Return a [`WasiWebSocketCtxView`] from mutable reference to self.
    fn websocket(&mut self) -> WasiWebSocketCtxView<'_>;
}

/// View into [`WebSocketCtx`] implementation and [`ResourceTable`].
pub struct WasiWebSocketCtxView<'a> {
    /// Mutable reference to the WASI WebSocket context.
    pub ctx: &'a mut dyn WebSocketCtx,

    /// Mutable reference to table used to manage resources.
    pub table: &'a mut ResourceTable,
}

/// A trait which provides internal WASI WebSocket context.
///
/// This is implemented by the resource-specific provider of WebSocket
/// functionality.
pub trait WebSocketCtx: Debug + Send + Sync + 'static {
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

/// `anyhow::Error` to `Error` mapping
impl From<anyhow::Error> for Error {
    fn from(err: anyhow::Error) -> Self {
        Self::Other(err.to_string())
    }
}

/// `ResourceTableError` to `Error` mapping
impl From<ResourceTableError> for Error {
    fn from(err: ResourceTableError) -> Self {
        Self::Other(err.to_string())
    }
}

/// `wasmtime::Error` to `Error` mapping
impl From<wasmtime::Error> for Error {
    fn from(err: wasmtime::Error) -> Self {
        Self::Other(err.to_string())
    }
}

/// Implementation of the `WebSocketView` trait for the store context.
#[macro_export]
macro_rules! qwasr_wasi_view {
    ($store_ctx:ty, $field_name:ident) => {
        impl qwasr_wasi_websocket::WebSocketView for $store_ctx {
            fn websocket(&mut self) -> qwasr_wasi_websocket::WasiWebSocketCtxView<'_> {
                qwasr_wasi_websocket::WasiWebSocketCtxView {
                    ctx: &mut self.$field_name,
                    table: &mut self.table,
                }
            }
        }
    };
}
