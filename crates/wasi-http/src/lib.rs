#![doc = include_str!("../README.md")]

//! # WASI Http Service
//!
//! This module implements a runtime service for `wasi:http`
//! (<https://github.com/WebAssembly/wasi-http>).

mod client_cert;
mod default_impl;
mod server;

use anyhow::Result;
pub use default_impl::{ConnectOptions, HttpDefault};
use omnia_core::{Grants, Host, HostCtx, Provides, Runtime, StoreCtx};
pub use server::{HttpHandler, OutgoingBody, ServerOptions};
use wasmtime::component::{HasData, Linker, ResourceTable};
use wasmtime_wasi_http::p3::bindings::http::{client, types};
pub use wasmtime_wasi_http::{
    Error as ErrorCode, RequestOptions, WasiBody, WasiHttpCtx, WasiHttpCtxView, WasiHttpHooks,
};

/// The `wasi:http` slot of a backend bundle: split borrows on the backend's
/// hooks and context into the linker-facing view.
///
/// A backend other than [`HttpDefault`] answers outbound requests through
/// its own [`WasiHttpHooks`].
pub trait HttpBorrow: Send {
    /// Borrow the `wasi:http` context as the linker-facing view, threading in
    /// the store's [`ResourceTable`] and the [`Grants`] of the store's guest.
    ///
    /// A backend that sends requests refuses one, and each redirect hop it
    /// follows, whose host the grants' [`hosts`](Grants::hosts) do not
    /// reach, with [`ErrorCode::HttpRequestDenied`], as [`HttpDefault`]
    /// does.
    fn as_view<'a>(
        &'a mut self, table: &'a mut ResourceTable, grants: &'a Grants,
    ) -> WasiHttpCtxView<'a>;
}

/// Host-side service for `wasi:http`.
#[derive(Debug)]
pub struct WasiHttp;

impl HasData for WasiHttp {
    type Data<'a> = WasiHttpCtxView<'a>;
}

impl HostCtx for WasiHttp {
    type Borrow<'a> = &'a mut dyn HttpBorrow;

    fn view<'a>(
        borrow: Self::Borrow<'a>, table: &'a mut ResourceTable, grants: &'a Grants,
    ) -> WasiHttpCtxView<'a> {
        borrow.as_view(table, grants)
    }
}

impl<B: Provides<Self>> Host<B> for WasiHttp {
    fn add_to_linker(linker: &mut Linker<StoreCtx<B>>) -> Result<()> {
        client::add_to_linker::<_, wasmtime_wasi_http::WasiHttp>(linker, StoreCtx::view::<Self>)?;
        types::add_to_linker::<_, wasmtime_wasi_http::WasiHttp>(linker, StoreCtx::view::<Self>)?;
        Ok(())
    }

    async fn run(runtime: &Runtime<B>) -> Result<()> {
        server::run(runtime).await
    }
}
