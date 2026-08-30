//! # WASI Config Host
//!
//! This module implements a host-side service for `wasi:config`.

mod default_impl;

use std::fmt::Debug;

pub use default_impl::ConfigDefault;
use omnia::{Host, HostCtx, Server, StoreView};
use wasmtime::component::{HasData, Linker, ResourceTable};
pub use wasmtime_wasi_config;
use wasmtime_wasi_config::WasiConfigVariables;

/// Host-side service for `wasi:config`.
#[derive(Debug)]
pub struct WasiConfig;

impl HasData for WasiConfig {
    type Data<'a> = wasmtime_wasi_config::WasiConfig<'a>;
}

// `wasi:config`'s linker-facing view is a shared borrow over the backend's
// variables — no resource table involved — so the `HostCtx` impl is
// hand-written rather than `wasi_view!`-generated.
impl HostCtx for WasiConfig {
    type Borrow<'a> = &'a dyn WasiConfigCtx;

    fn view<'a>(
        borrow: Self::Borrow<'a>, _table: &'a mut ResourceTable,
    ) -> wasmtime_wasi_config::WasiConfig<'a> {
        wasmtime_wasi_config::WasiConfig::from(borrow.get_config())
    }
}

impl<T> Host<T> for WasiConfig
where
    T: StoreView<Self> + 'static,
{
    fn add_to_linker(linker: &mut Linker<T>) -> anyhow::Result<()> {
        Ok(wasmtime_wasi_config::add_to_linker(linker, T::view)?)
    }
}

impl<B> Server<B> for WasiConfig {}

/// A trait which provides internal WASI Config context.
pub trait WasiConfigCtx: Debug + Send + Sync + 'static {
    /// Get the configuration variables.
    fn get_config(&self) -> &WasiConfigVariables;
}
