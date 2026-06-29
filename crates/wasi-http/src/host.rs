//! #WASI HTTP Host
//!
//! This module implements a host-side service for `wasi:http`

mod default_impl;
mod server;

use anyhow::Result;
pub use default_impl::HttpDefault;
use omnia::{Host, Runtime, Server};
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

impl<R> Server<R> for WasiHttp
where
    R: Runtime,
    R::StoreCtx: WasiHttpView,
{
    const IS_SERVER: bool = true;

    async fn run(&self, state: &R) -> Result<()> {
        server::run(state).await
    }
}

/// Generates the bundle's [`omnia::HasHttp`] impl for a `runtime!` deployment.
///
/// `wasi:http`'s view trait (`WasiHttpView`) is foreign, so the
/// `WasiHttpView for omnia::StoreCtx<B>` impl lives in `omnia`; this only wires
/// the bundle field's [`HttpDefault::as_view`] through `HasHttp`.
#[macro_export]
macro_rules! omnia_wasi_view {
    ($bundle:ty, $field_name:ident) => {
        impl omnia::HasHttp for $bundle {
            fn http_view<'a>(
                &'a mut self, table: &'a mut omnia::wasmtime_wasi::ResourceTable,
            ) -> omnia_wasi_http::WasiHttpCtxView<'a> {
                self.$field_name.as_view(table)
            }
        }
    };
}
