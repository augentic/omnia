//! #WASI HTTP Host
//!
//! This module implements a host-side service for `wasi:http`

mod default_impl;

use std::fmt::Debug;

use anyhow::Result;
pub use default_impl::ConfigDefault;
use qwasr::{Host, Server, State};
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
    fn add_to_linker(linker: &mut Linker<T>) -> Result<()> {
        wasmtime_wasi_config::add_to_linker(linker, T::config)
    }
}

impl<S> Server<S> for WasiConfig where S: State {}

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

/// Implementation of the `WasiConfigView` trait for the store context.
#[macro_export]
macro_rules! qwasr_wasi_view {
    ($store_ctx:ty, $field_name:ident) => {
        impl qwasr_wasi_config::WasiConfigView for $store_ctx {
            fn config(&mut self) -> qwasr_wasi_config::wasmtime_wasi_config::WasiConfig<'_> {
                let vars = qwasr_wasi_config::WasiConfigCtx::get_config(&self.$field_name);
                qwasr_wasi_config::wasmtime_wasi_config::WasiConfig::from(vars)
            }
        }
    };
}
