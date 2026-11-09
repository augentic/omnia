//! # Host implementation for WASI Vault Service
//!
//! This module implements the host-side logic for the WASI Vault service.

mod default_impl;
mod resource;
mod vault_impl;

mod generated {
    pub use self::omnia::vault::vault::Error;
    pub use super::LockerProxy;

    wasmtime::component::bindgen!({
        world: "imports",
        path: "wit",
        imports: {
            default: store | tracing | trappable,
        },
        with: {
            "omnia:vault/vault.locker": LockerProxy,
        },
        trappable_error_type: {
            "omnia:vault/vault.error" => Error,
        },
    });
}

use std::fmt::Debug;

pub use default_impl::VaultDefault;
pub use omnia_backend::vault::{Locker, VaultClient};
pub use omnia_core::FutureResult;
use omnia_core::{Host, HostCtx, Server, StoreView};
use wasmtime::component::Linker;

use self::generated::omnia::vault::vault;
use crate::host::generated::Error;
pub use crate::host::resource::*;

/// Result type for vault operations.
pub type Result<T> = std::result::Result<T, Error>;

/// Host-side service for `wasi:vault`.
#[derive(Debug, HostCtx)]
pub struct WasiVault;

impl<T> Host<T> for WasiVault
where
    T: StoreView<Self> + 'static,
{
    fn add_to_linker(linker: &mut Linker<T>) -> anyhow::Result<()> {
        Ok(vault::add_to_linker::<_, Self>(linker, T::view)?)
    }
}

impl<B> Server<B> for WasiVault {}

omnia_core::host_error!(Error, Other);
