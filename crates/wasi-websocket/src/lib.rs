#![doc = include_str!("../README.md")]

//! # WASI WebSocket Service
//!
//! Serves `omnia:websocket` to guests over a [`WebSocketClient`], and runs its
//! event trigger.

mod broadcast_impl;
mod connections_impl;
mod default_impl;
mod resource;
mod server;
mod types_impl;

mod generated {
    #![allow(missing_docs, reason = "wasmtime bindgen output")]

    pub use omnia_wasi_types::Error;

    pub use crate::resource::ClientProxy;

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
            "omnia:websocket/types.client": ClientProxy,
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
use omnia_core::{Host, HostCtx, Provides, Runtime, StoreCtx};
use wasmtime::component::Linker;

pub use self::default_impl::{ConnectOptions, WebSocketDefault};
use self::generated::omnia::websocket::{broadcast, connections, types};
pub use self::generated::{Error, Websocket};
pub use self::resource::*;
pub use self::server::WebSocketHandler;

/// Result type for WebSocket operations.
pub type Result<T> = std::result::Result<T, Error>;

/// Host-side service for `omnia:websocket`.
#[derive(Clone, Debug, HostCtx)]
#[host_ctx(client = WebSocketClient)]
pub struct WasiWebSocket;

impl<B: Provides<Self>> Host<B> for WasiWebSocket {
    fn add_to_linker(linker: &mut Linker<StoreCtx<B>>) -> anyhow::Result<()> {
        omnia_wasi_types::types::add_to_linker::<_, Self>(linker, StoreCtx::view::<Self>)?;
        types::add_to_linker::<_, Self>(linker, StoreCtx::view::<Self>)?;
        connections::add_to_linker::<_, Self>(linker, StoreCtx::view::<Self>)?;
        Ok(broadcast::add_to_linker::<_, Self>(linker, StoreCtx::view::<Self>)?)
    }

    async fn run(runtime: &Runtime<B>) -> anyhow::Result<()> {
        server::run(runtime).await
    }
}

/// Service contract for `omnia:websocket` backends.
///
/// This is implemented by the resource-specific provider of WebSocket
/// functionality.
///
/// Event construction lives on the host's [`Event`] struct; backends only
/// translate at the [`Endpoint`] seam.
pub trait WebSocketClient: Debug + Send + Sync + 'static {
    /// Open a WebSocket endpoint handle.
    ///
    /// # Errors
    ///
    /// Returns an error if the connection fails.
    fn endpoint(&self) -> FutureResult<Arc<dyn Endpoint>>;
}
