//! # WASI WebSocket Service
//!
//! This module implements a runtime server for websocket

mod client_impl;
mod default_impl;
mod resource;
mod server;
mod types_impl;

mod generated {
    #![allow(missing_docs, reason = "wasmtime bindgen output")]

    pub use self::omnia::websocket::types::Error;
    pub use crate::host::resource::{ClientProxy, Event};

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
            "omnia:websocket/types.event": Event,
        },
        trappable_error_type: {
            "omnia:websocket/types.error" => Error,
        },
    });
}

use std::fmt::Debug;
use std::sync::Arc;

pub use omnia_core::FutureResult;
use omnia_core::{Host, HostCtx, Provides, Runtime, StoreCtx};
use wasmtime::component::Linker;

pub use self::default_impl::{ConnectOptions, WebSocketDefault};
pub use self::generated::Duplex;
pub use self::generated::omnia::websocket::types::Error;
use self::generated::omnia::websocket::{client, types as generated_types};
pub use self::resource::*;
pub use self::server::WebSocketHandler;

/// Result type for WebSocket operations.
pub type Result<T> = std::result::Result<T, Error>;

/// Host-side service for `wasi:websocket`.
#[derive(Clone, Debug, HostCtx)]
#[host_ctx(client = WasiWebSocketCtx)]
pub struct WasiWebSocket;

impl<B: Provides<Self>> Host<B> for WasiWebSocket {
    fn add_to_linker(linker: &mut Linker<StoreCtx<B>>) -> anyhow::Result<()> {
        client::add_to_linker::<_, Self>(linker, StoreCtx::view::<Self>)?;
        Ok(generated_types::add_to_linker::<_, Self>(linker, StoreCtx::view::<Self>)?)
    }

    async fn run(runtime: &Runtime<B>) -> anyhow::Result<()> {
        server::run(runtime).await
    }
}

/// A trait which provides internal WASI WebSocket context.
///
/// This is implemented by the resource-specific provider of WebSocket
/// functionality.
///
/// Event construction lives on the host's [`Event`] struct; backends only
/// translate at the [`Client`] seam.
pub trait WasiWebSocketCtx: Debug + Send + Sync + 'static {
    /// Connect to the WebSocket service and return a socket.
    ///
    /// # Errors
    ///
    /// Returns an error if the connection fails.
    fn connect(&self) -> FutureResult<Arc<dyn Client>>;
}

omnia_core::host_error!(Error, Other);
