//! # WASI WebSocket Service
//!
//! This module implements a runtime server for websocket

mod client_impl;
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

#[expect(deprecated, reason = "re-exporting the deprecated alias")]
pub use omnia_backend::websocket::Client;
pub use omnia_backend::websocket::{
    ConnectOptions, Endpoint, Event, Events, WebSocketClient, WebSocketDefault,
};
pub use omnia_core::FutureResult;
use omnia_core::{Host, Runtime, Server, StoreCtx, StoreView};
use wasmtime::component::{HasData, Linker};

pub use self::generated::Duplex;
pub use self::generated::omnia::websocket::types::Error;
use self::generated::omnia::websocket::{client, types as generated_types};
pub use self::resource::*;
pub use self::server::WebSocketHandler;

/// Result type for WebSocket operations.
pub type Result<T> = std::result::Result<T, Error>;

/// Host-side service for `wasi:websocket`.
#[derive(Clone, Debug)]
pub struct WasiWebSocket;

impl HasData for WasiWebSocket {
    type Data<'a> = WasiWebSocketView<'a>;
}

impl<T> Host<T> for WasiWebSocket
where
    T: StoreView<Self> + 'static,
{
    fn add_to_linker(linker: &mut Linker<T>) -> anyhow::Result<()> {
        client::add_to_linker::<_, Self>(linker, T::view)?;
        Ok(generated_types::add_to_linker::<_, Self>(linker, T::view)?)
    }
}

impl<B> Server<B> for WasiWebSocket
where
    B: Clone + Send + Sync + 'static,
    StoreCtx<B>: StoreView<Self>,
{
    async fn run(&self, state: &Runtime<B>) -> anyhow::Result<()> {
        server::run(state).await
    }
}

omnia_core::host_error!(Error, Other);
omnia_core::wasi_view!(WebSocket);
