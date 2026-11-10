//! # WASI WebSocket Service
//!
//! This module implements a runtime server for websocket

mod client_impl;
mod default_impl;
mod resource;
mod server;
mod types_impl;

mod generated {
    #![expect(missing_docs, reason = "wasmtime bindgen output")]

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

pub use default_impl::{ConnectOptions, WebSocketDefault};
pub use omnia_backend::websocket::{Endpoint, Event, Events, WebSocketClient};
pub use omnia_core::FutureResult;
use omnia_core::{Backends, Host, HostCtx, Provides, Runtime, StoreCtx};
use wasmtime::component::Linker;

pub use self::generated::Duplex;
pub use self::generated::omnia::websocket::types::Error;
use self::generated::omnia::websocket::{client, types as generated_types};
pub use self::resource::*;
pub use self::server::WebSocketHandler;

/// Result type for WebSocket operations.
pub type Result<T> = std::result::Result<T, Error>;

/// Host-side service for `omnia:websocket`.
#[derive(Clone, Debug, HostCtx)]
#[host_ctx(client = WebSocketClient)]
pub struct WasiWebSocket;

impl<B: Backends + Provides<Self>> Host<B> for WasiWebSocket {
    fn add_to_linker(linker: &mut Linker<StoreCtx<B>>) -> anyhow::Result<()> {
        client::add_to_linker::<_, Self>(linker, StoreCtx::view::<Self>)?;
        Ok(generated_types::add_to_linker::<_, Self>(linker, StoreCtx::view::<Self>)?)
    }

    async fn run(runtime: &Runtime<B>) -> anyhow::Result<()> {
        server::run(runtime).await
    }
}

omnia_core::host_error!(Error, Other);
