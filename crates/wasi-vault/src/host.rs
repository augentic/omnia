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
use std::sync::Arc;

pub use omnia::FutureResult;
use omnia::{Host, Server};
use wasmtime::component::{HasData, Linker, ResourceTableError};

use self::generated::omnia::vault::vault;
pub use crate::host::default_impl::VaultDefault;
use crate::host::generated::Error;
pub use crate::host::resource::*;

/// Result type for  vault operations.
pub type Result<T> = anyhow::Result<T, Error>;

/// Host-side service for `wasi:vault`.
#[derive(Debug)]
pub struct WasiVault;

impl HasData for WasiVault {
    type Data<'a> = WasiVaultCtxView<'a>;
}

impl<T> Host<T> for WasiVault
where
    T: WasiVaultView + 'static,
{
    fn add_to_linker(linker: &mut Linker<T>) -> anyhow::Result<()> {
        Ok(vault::add_to_linker::<_, Self>(linker, T::vault)?)
    }
}

impl<B> Server<B> for WasiVault {}

/// A trait which provides internal WASI Vault context.
///
/// This is implemented by the resource-specific provider of Vault
/// functionality.
pub trait WasiVaultCtx: Debug + Send + Sync + 'static {
    /// Open a locker.
    fn open_locker(&self, identifier: String) -> FutureResult<Arc<dyn Locker>>;
}

impl From<anyhow::Error> for Error {
    fn from(err: anyhow::Error) -> Self {
        Self::Other(err.to_string())
    }
}

impl From<ResourceTableError> for Error {
    fn from(err: ResourceTableError) -> Self {
        Self::Other(err.to_string())
    }
}

omnia::wasi_view!(Vault);
