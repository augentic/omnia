//! # WASI HTTP Host
//!
//! This module implements a host-side service for `wasi:http`.

pub mod client_cert;
mod default_impl;
mod server;

use std::fmt::{self, Debug};

use anyhow::Result;
pub use default_impl::{ConnectOptions, HttpDefault};
use omnia_core::{Capability, Host, HostCtx, Provides, Runtime, Serve, SlotHost, StoreCtx};
pub use server::{HttpHandler, OutgoingBody};
use wasmtime::component::{HasData, Linker, ResourceTable};
use wasmtime_wasi_http::p3::bindings::http::{client, types};
pub use wasmtime_wasi_http::{WasiHttpCtx, WasiHttpCtxView};

/// The `wasi:http` slot of a backend bundle: split borrows on the backend's
/// hooks and context into the linker-facing view.
pub trait HttpBorrow: Send + Sync + 'static {
    /// Borrow the `wasi:http` context as the linker-facing view, threading in
    /// the store's [`ResourceTable`].
    fn as_view<'a>(&'a mut self, table: &'a mut ResourceTable) -> WasiHttpCtxView<'a>;

    /// Clones the backend for another store.
    fn fresh(&self) -> Box<dyn HttpBorrow>;
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

impl Capability for WasiHttp {
    type Slot = HttpSlot;

    const NAME: &'static str = "wasi:http";
}

impl<T: HttpBorrow + Clone> Serve<T> for WasiHttp {
    fn slot(backend: &T) -> HttpSlot {
        HttpSlot(Box::new(backend.clone()))
    }
}

impl SlotHost for WasiHttp {
    fn lend(slot: &mut HttpSlot) -> &mut dyn HttpBorrow {
        &mut *slot.0
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

/// The [`WasiHttp`] slot: one `wasi:http` backend, cloned for every store.
pub struct HttpSlot(Box<dyn HttpBorrow>);

impl Clone for HttpSlot {
    fn clone(&self) -> Self {
        Self(self.0.fresh())
    }
}

impl Debug for HttpSlot {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HttpSlot").finish_non_exhaustive()
    }
}
