//! # WASI HTTP Host
//!
//! This module implements a host-side service for `wasi:http`.

pub mod client_cert;
mod default_impl;
mod server;

use std::fmt::Debug;

use anyhow::Result;
pub use default_impl::{ConnectOptions, HttpDefault};
use omnia_core::{Backends, Host, HostCtx, Provides, Runtime, StoreCtx};
pub use server::{HttpHandler, OutgoingBody};
use wasmtime::component::{HasData, Linker, ResourceTable};
use wasmtime_wasi_http::p3::bindings::http::{client, types};
pub use wasmtime_wasi_http::{WasiHttpCtx, WasiHttpCtxView};

/// The `wasi:http` slot of a backend bundle: split borrows on the backend's
/// hooks and context into the linker-facing view.
pub trait HttpBorrow: Send {
    /// Borrow the `wasi:http` context as the linker-facing view, threading in
    /// the store's [`ResourceTable`].
    fn as_view<'a>(&'a mut self, table: &'a mut ResourceTable) -> WasiHttpCtxView<'a>;
}

/// Host-side service for `wasi:http`.
#[derive(Debug)]
pub struct WasiHttp;

impl HasData for WasiHttp {
    type Data<'a> = WasiHttpCtxView<'a>;
}

impl HostCtx for WasiHttp {
    type Borrow<'a> = &'a mut dyn HttpBorrow;

    fn view<'a>(borrow: Self::Borrow<'a>, table: &'a mut ResourceTable) -> WasiHttpCtxView<'a> {
        borrow.as_view(table)
    }
}

impl<B: Backends + Provides<Self>> Host<B> for WasiHttp {
    fn add_to_linker(linker: &mut Linker<StoreCtx<B>>) -> Result<()> {
        client::add_to_linker::<_, wasmtime_wasi_http::WasiHttp>(linker, StoreCtx::view::<Self>)?;
        types::add_to_linker::<_, wasmtime_wasi_http::WasiHttp>(linker, StoreCtx::view::<Self>)?;
        Ok(())
    }

    async fn run(runtime: &Runtime<B>) -> Result<()> {
        server::run(runtime).await
    }
}
