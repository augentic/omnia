//! #WASI HTTP Host
//!
//! This module implements a host-side service for `wasi:http`

mod default_impl;
mod server;

use anyhow::Result;
pub use default_impl::HttpDefault;
use qwasr::{Host, Server, State};
use wasmtime::component::Linker;
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

impl<S> Server<S> for WasiHttp
where
    S: State,
    S::StoreCtx: WasiHttpView,
{
    async fn run(&self, state: &S) -> Result<()> {
        server::serve(state).await
    }
}

/// Implementation of the `WasiHttpView` trait for the store context.
#[macro_export]
macro_rules! qwasr_wasi_view {
    ($store_ctx:ty, $field_name:ident) => {
        impl qwasr_wasi_http::WasiHttpView for $store_ctx {
            fn http(&mut self) -> qwasr_wasi_http::WasiHttpCtxView<'_> {
                qwasr_wasi_http::WasiHttpCtxView {
                    ctx: &mut self.$field_name,
                    table: &mut self.table,
                }
            }
        }
    };
}
