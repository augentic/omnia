//! # Host implementation for WASI Vault Service
//!
//! This module implements the host-side logic for the WASI Vault service.

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

pub use omnia_backend::vault::{Locker, VaultClient, VaultDefault};
pub use omnia_core::FutureResult;
use omnia_core::{Host, Server, StoreView};
use wasmtime::component::{HasData, Linker};

use self::generated::omnia::vault::vault;
use crate::host::generated::Error;
pub use crate::host::resource::*;

/// Result type for vault operations.
pub type Result<T> = std::result::Result<T, Error>;

/// Host-side service for `wasi:vault`.
#[derive(Debug)]
pub struct WasiVault;

impl HasData for WasiVault {
    type Data<'a> = WasiVaultView<'a>;
}

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
omnia_core::wasi_view!(Vault);
