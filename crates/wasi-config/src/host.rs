//! # WASI Config Host
//!
//! This module implements a host-side service for `wasi:config`.

mod default_impl;

use std::fmt::Debug;

pub use default_impl::ConfigDefault;
use omnia::{Host, Runtime, Server};
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

impl<R> Server<R> for WasiConfig where R: Runtime {}

/// A trait which provides internal WASI Config state.
///
/// This is implemented by the `T` in `Linker<T>` — a single type shared across
/// all WASI components for the runtime build.
pub trait WasiConfigView: Send {
    /// Return a [`WasiConfig`] from mutable reference to self.
    fn config(&mut self) -> wasmtime_wasi_config::WasiConfig<'_>;
}

/// A trait which provides internal WASI Config context.
///
/// This is implemented by the resource-specific provider of Config
/// functionality.
pub trait WasiConfigCtx: Debug + Send + Sync + 'static {
    /// Get the configuration variables.
    fn get_config(&self) -> &WasiConfigVariables;
}

/// A backend bundle that can yield the `wasi:config` backend for a store.
///
/// The blanket [`WasiConfigView`] impl below turns this accessor into the
/// linker-facing view on `omnia::StoreCtx<B>`; the `runtime!` macro generates
/// the bundle-side impl via [`omnia_wasi_view!`]. The accessor borrows shared
/// because [`WasiConfigCtx::get_config`] takes `&self`.
pub trait HasConfig: Send {
    /// Borrow the `wasi:config` backend context.
    fn config_ctx(&self) -> &dyn WasiConfigCtx;
}

impl<B: HasConfig + Send + 'static> WasiConfigView for omnia::StoreCtx<B> {
    fn config(&mut self) -> wasmtime_wasi_config::WasiConfig<'_> {
        wasmtime_wasi_config::WasiConfig::from(self.backends.config_ctx().get_config())
    }
}

/// Generates the bundle's [`HasConfig`] impl for a `runtime!` deployment.
#[macro_export]
macro_rules! omnia_wasi_view {
    ($bundle:ty, $field_name:ident) => {
        impl $crate::HasConfig for $bundle {
            fn config_ctx(&self) -> &dyn $crate::WasiConfigCtx {
                &self.$field_name
            }
        }
    };
}
