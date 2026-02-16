//! # WASI WebSocket Service
//!
//! This module implements a runtime server for websocket

mod default_impl;
mod resource;
mod server;
mod store_impl;
mod types;

mod generated {

    pub use super::resource::ServerProxy;

    wasmtime::component::bindgen!({
        world: "websocket",
        path: "wit",
        imports: {
            default: store | tracing | trappable,
        },
        trappable_error_type: {
            "wasi:websocket/types.error" => anyhow::Error,
        },
        with: {
            "wasi:websocket/server.server": ServerProxy,
        },
    });
}

use std::fmt::Debug;
use std::sync::Arc;

use anyhow::Result;
use qwasr::{Host, Server, State};
use server::run_server;
use store_impl::FutureResult;
use wasmtime::component::{HasData, Linker};
use wasmtime_wasi::ResourceTable;

pub use self::default_impl::WebSocketDefault;
use self::generated::wasi::websocket::{self, types as generated_types};

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
    fn add_to_linker(linker: &mut Linker<T>) -> Result<()> {
        Ok(store::add_to_linker::<_, Self>(linker, T::websocket)?)
    }
}

impl<S> Server<S> for WasiWebSocket
where
    S: State,
    S::StoreCtx: WebSocketView,
{
    /// Provide http proxy service the specified wasm component.
    /// ``state`` will be used at a later time to provide resource access to guest handlers
    async fn run(&self, state: &S) -> Result<()> {
        run_server(state).await
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
    pub ctx: &'a dyn WebSocketCtx,

    /// Mutable reference to table used to manage resources.
    pub table: &'a mut ResourceTable,
}

/// A trait which provides internal WASI WebSocket context.
///
/// This is implemented by the resource-specific provider of WebSocket
/// functionality.
pub trait WebSocketCtx: Debug + Send + Sync + 'static {
    /// Start a WebSocket server.
    fn serve(&self) -> FutureResult<Arc<dyn resource::Server>>;
}

impl generated_types::Host for WasiWebSocketCtxView<'_> {
    fn convert_error(&mut self, err: anyhow::Error) -> wasmtime::Result<String> {
        Ok(err.to_string())
    }
}

/// Implementation of the `WebSocketView` trait for the store context.
#[macro_export]
macro_rules! qwasr_wasi_view {
    ($store_ctx:ty, $field_name:ident) => {
        impl qwasr_wasi_websocket::WebSocketView for $store_ctx {
            fn websocket(&mut self) -> qwasr_wasi_websocket::WasiWebSocketCtxView<'_> {
                qwasr_wasi_websocket::WasiWebSocketCtxView {
                    ctx: &self.$field_name,
                    table: &mut self.table,
                }
            }
        }
    };
}
