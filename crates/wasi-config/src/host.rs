//! # WASI Config Host
//!
//! This module implements a host-side service for `wasi:config`.

mod default_impl;

mod generated {
    #![allow(missing_docs, reason = "wasmtime bindgen output")]

    pub use self::wasi::config::store::Error;

    wasmtime::component::bindgen!({
        world: "imports",
        path: "wit",
        imports: {
            default: async | tracing | trappable,
        },
        trappable_error_type: {
            "wasi:config/store.error" => Error,
        },
    });
}

use std::fmt::Debug;

pub use default_impl::ConfigDefault;
pub use omnia_backend::config::ConfigClient;
pub use omnia_core::FutureResult;
use omnia_core::{Host, Server, StoreView};
use wasmtime::component::{HasData, Linker};

use self::generated::wasi::config::store::{self, Error, Host as StoreHost};

/// Result type for config operations.
pub type Result<T> = std::result::Result<T, Error>;

/// Host-side service for `wasi:config`.
#[derive(Debug)]
pub struct WasiConfig;

impl HasData for WasiConfig {
    type Data<'a> = WasiConfigView<'a>;
}

impl<T> Host<T> for WasiConfig
where
    T: StoreView<Self> + 'static,
{
    fn add_to_linker(linker: &mut Linker<T>) -> anyhow::Result<()> {
        Ok(store::add_to_linker::<_, Self>(linker, T::view)?)
    }
}

impl<B> Server<B> for WasiConfig {}

impl StoreHost for WasiConfigView<'_> {
    async fn get(&mut self, key: String) -> Result<Option<String>> {
        Ok(self.client.get(key).await?)
    }

    async fn get_all(&mut self) -> Result<Vec<(String, String)>> {
        Ok(self.client.get_all().await?)
    }

    fn convert_error(&mut self, err: Error) -> wasmtime::Result<Error> {
        Ok(err)
    }
}

omnia_core::host_error!(Error, Upstream);
omnia_core::wasi_view!(Config);
