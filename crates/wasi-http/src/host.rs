//! #WASI HTTP Host
//!
//! This module implements a host-side service for `wasi:http`

mod default_impl;
mod server;

use anyhow::Result;
pub use default_impl::HttpDefault;
use omnia::{Host, Runtime, Server, StoreCtx};
use wasmtime::component::Linker;
pub use wasmtime_wasi_http::WasiHttpCtx;
pub use wasmtime_wasi_http::p3::{WasiHttpCtxView, WasiHttpView};

/// Host-side service for `wasi:http`.
#[derive(Debug)]
pub struct WasiHttp;

impl<T> Host<T> for WasiHttp
where
    T: WasiHttpView + 'static,
{
    fn add_to_linker(linker: &mut Linker<T>) -> Result<()> {
        Ok(wasmtime_wasi_http::p3::add_to_linker(linker)?)
    }
}

impl<B> Server<B> for WasiHttp
where
    B: Clone + Send + Sync + 'static,
    StoreCtx<B>: WasiHttpView,
{
    const IS_SERVER: bool = true;

    async fn run(&self, state: &Runtime<B>) -> Result<()> {
        server::run(state).await
    }
}
