//! # Host implementation for WASI Vault Service
//!
//! This module implements the host-side logic for the WASI Vault service.

pub mod default_impl;
mod resource;
mod vault_impl;

mod generated {

    pub use self::wasi::vault::vault::Error;
    pub use super::LockerProxy;

    wasmtime::component::bindgen!({
        world: "vault",
        path: "wit",
        imports: {
            default: store | tracing | trappable,
        },
        with: {
            "wasi:vault/vault.locker": LockerProxy,
        },
        trappable_error_type: {
            "wasi:vault/vault.error" => Error,
        },
    });
}

use std::fmt::Debug;
use std::sync::Arc;

pub use qwasr::FutureResult;
use qwasr::{Host, Server, State};
use wasmtime::component::{HasData, Linker, ResourceTableError};
use wasmtime_wasi::ResourceTable;

use self::generated::wasi::vault::vault;
pub use crate::host::default_impl::VaultDefault;
use crate::host::generated::Error;
pub use crate::host::resource::*;

/// Result type for  vault operations.
pub type Result<T, E = Error> = anyhow::Result<T, E>;

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

impl<S> Server<S> for WasiVault where S: State {}

/// A trait which provides internal WASI Vault state.
///
/// This is implemented by the `T` in `Linker<T>` — a single type shared across
/// all WASI components for the runtime build.
pub trait WasiVaultView: Send {
    /// Return a [`WasiVaultCtxView`] from mutable reference to self.
    fn vault(&mut self) -> WasiVaultCtxView<'_>;
}

/// View into [`WasiVaultCtx`] implementation and [`ResourceTable`].
pub struct WasiVaultCtxView<'a> {
    /// Mutable reference to the WASI Vault context.
    pub ctx: &'a mut dyn WasiVaultCtx,

    /// Mutable reference to table used to manage resources.
    pub table: &'a mut ResourceTable,
}

/// A trait which provides internal WASI Vault context.
///
/// This is implemented by the resource-specific provider of Vault
/// functionality.
pub trait WasiVaultCtx: Debug + Send + Sync + 'static {
    /// Open a locker.
    fn open_locker(&self, identifier: String) -> FutureResult<Arc<dyn Locker>>;
}

/// `anyhow::Error` to `Error` mapping
impl From<anyhow::Error> for Error {
    fn from(err: anyhow::Error) -> Self {
        Self::Other(err.to_string())
    }
}

/// `ResourceTableError` to `Error` mapping
impl From<ResourceTableError> for Error {
    fn from(err: ResourceTableError) -> Self {
        Self::Other(err.to_string())
    }
}

/// Implementation of the `WasiVaultView` trait for the store context.
#[macro_export]
macro_rules! qwasr_wasi_view {
    ($store_ctx:ty, $field_name:ident) => {
        impl qwasr_wasi_vault::WasiVaultView for $store_ctx {
            fn vault(&mut self) -> qwasr_wasi_vault::WasiVaultCtxView<'_> {
                qwasr_wasi_vault::WasiVaultCtxView {
                    ctx: &mut self.$field_name,
                    table: &mut self.table,
                }
            }
        }
    };
}
