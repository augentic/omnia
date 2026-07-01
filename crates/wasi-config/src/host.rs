//! # WASI Config Host
//!
//! This module implements a host-side service for `wasi:config`.

mod default_impl;

use std::fmt::Debug;

pub use default_impl::ConfigDefault;
use omnia::{Host, Server};
use wasmtime::component::{HasData, Linker};
pub use wasmtime_wasi_config;
use wasmtime_wasi_config::WasiConfigVariables;

/// Host-side service for `wasi:config`.
#[derive(Debug)]
pub struct WasiConfig;

impl HasData for WasiConfig {
    type Data<'a> = wasmtime_wasi_config::WasiConfig<'a>;
}

impl<T> Host<T> for WasiConfig
where
    T: WasiConfigView + 'static,
{
    fn add_to_linker(linker: &mut Linker<T>) -> anyhow::Result<()> {
        Ok(wasmtime_wasi_config::add_to_linker(linker, T::config)?)
    }
}

impl<B> Server<B> for WasiConfig {}

/// A trait which provides internal WASI Config state.
/// Implemented by the `T` in `Linker<T>` during the runtime build.
pub trait WasiConfigView: Send {
    /// Return a [`WasiConfig`] from mutable reference to self.
    fn config(&mut self) -> wasmtime_wasi_config::WasiConfig<'_>;
}

/// A trait which provides internal WASI Config context.
pub trait WasiConfigCtx: Debug + Send + Sync + 'static {
    /// Get the configuration variables.
    fn get_config(&self) -> &WasiConfigVariables;
}

/// A backend bundle that can yield the `wasi:config` backend context.
pub trait HasConfig: Send {
    /// Borrow the `wasi:config` backend context.
    fn config_ctx(&self) -> &dyn WasiConfigCtx;
}

impl<B: HasConfig + Send + 'static> WasiConfigView for omnia::StoreCtx<B> {
    fn config(&mut self) -> wasmtime_wasi_config::WasiConfig<'_> {
        wasmtime_wasi_config::WasiConfig::from(self.backends.config_ctx().get_config())
    }
}
