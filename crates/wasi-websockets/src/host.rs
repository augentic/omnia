//! # WASI Websockets Service
//!
//! This module implements a runtime server for websockets

mod default_impl;
mod resource;
mod server;
mod store_impl;
mod types;

mod generated {
    #![allow(clippy::trait_duplication_in_bounds)]

    pub use super::resource::ServerProxy;

    wasmtime::component::bindgen!({
        world: "websockets",
        path: "wit",
        imports: {
            default: store | tracing | trappable,
        },
        trappable_error_type: {
            "wasi:websockets/types.error" => anyhow::Error,
        },
        with: {
            "wasi:websockets/store.server": ServerProxy,
        },
    });
}

use std::fmt::Debug;
use std::sync::Arc;

use anyhow::Result;
use server::run_server;
use store_impl::FutureResult;
use wasmtime::component::{HasData, Linker};
use wasmtime_wasi::ResourceTable;
use yetti::{Host, Server, State};

pub use self::default_impl::WebSocketsDefault;
use self::generated::wasi::websockets::{store, types as generated_types};

#[derive(Clone, Debug)]
pub struct WasiWebSockets;

impl HasData for WasiWebSockets {
    type Data<'a> = WasiWebSocketsCtxView<'a>;
}

impl<T> Host<T> for WasiWebSockets
where
    T: WebSocketsView + 'static,
{
    fn add_to_linker(linker: &mut Linker<T>) -> Result<()> {
        store::add_to_linker::<_, Self>(linker, T::websockets)
    }
}

impl<S> Server<S> for WasiWebSockets
where
    S: State,
    S::StoreCtx: WebSocketsView,
{
    /// Provide http proxy service the specified wasm component.
    /// ``state`` will be used at a later time to provide resource access to guest handlers
    async fn run(&self, state: &S) -> Result<()> {
        run_server(state).await
    }
}

/// A trait which provides internal WASI WebSockets state.
///
/// This is implemented by the `T` in `Linker<T>` — a single type shared across
/// all WASI components for the runtime build.
pub trait WebSocketsView: Send {
    fn websockets(&mut self) -> WasiWebSocketsCtxView<'_>;
}

/// View into [`WebSocketsCtx`] implementation and [`ResourceTable`].
pub struct WasiWebSocketsCtxView<'a> {
    /// Mutable reference to the WASI WebSockets context.
    pub ctx: &'a dyn WebSocketsCtx,

    /// Mutable reference to table used to manage resources.
    pub table: &'a mut ResourceTable,
}

/// A trait which provides internal WASI WebSockets context.
///
/// This is implemented by the resource-specific provider of WebSockets
/// functionality.
pub trait WebSocketsCtx: Debug + Send + Sync + 'static {
    fn serve(&self) -> FutureResult<Arc<dyn resource::Server>>;
}

impl generated_types::Host for WasiWebSocketsCtxView<'_> {
    fn convert_error(&mut self, err: anyhow::Error) -> Result<String> {
        Ok(err.to_string())
    }
}

#[macro_export]
macro_rules! wasi_view {
    ($store_ctx:ty, $field_name:ident) => {
        impl yetti_wasi_websockets::WebSocketsView for $store_ctx {
            fn websockets(&mut self) -> yetti_wasi_websockets::WasiWebSocketsCtxView<'_> {
                yetti_wasi_websockets::WasiWebSocketsCtxView {
                    ctx: &self.$field_name,
                    table: &mut self.table,
                }
            }
        }
    };
}
