//! # WASI Config Host
//!
//! This module implements a host-side service for `wasi:config`.

mod default_impl;

mod generated {
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
use omnia_core::{Host, HostCtx, HostView, Provides, StoreCtx};
use wasmtime::component::Linker;

use self::generated::wasi::config::store::{self, Error, Host as StoreHost};

/// Result type for config operations.
pub type Result<T> = std::result::Result<T, Error>;

/// Host-side service for `wasi:config`.
#[derive(Debug, HostCtx)]
#[host_ctx(client = ConfigClient)]
pub struct WasiConfig;

omnia_backend::capability!(WasiConfig, "wasi:config", ConfigClient);

impl<B: Provides<Self>> Host<B> for WasiConfig {
    fn add_to_linker(linker: &mut Linker<StoreCtx<B>>) -> anyhow::Result<()> {
        Ok(store::add_to_linker::<_, Self>(linker, StoreCtx::view::<Self>)?)
    }
}

omnia_core::host_error!(Error, Upstream);

impl StoreHost for HostView<'_, dyn ConfigClient> {
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
