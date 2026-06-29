//! # Host implementation for WASI Vault Service
//!
//! This module implements the host-side logic for the WASI Vault service.

pub mod default_impl;
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
use omnia::{Host, Runtime, Server};
use wasmtime::component::{HasData, Linker, ResourceTable, ResourceTableError};

use self::generated::omnia::vault::vault;
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

impl<R> Server<R> for WasiVault where R: Runtime {}

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

/// A backend bundle that can yield the `wasi:vault` backend for a store.
///
/// The blanket [`WasiVaultView`] impl below turns this accessor into the
/// linker-facing view on `omnia::StoreCtx<B>`; the `runtime!` macro generates
/// the bundle-side impl via [`omnia_wasi_view!`].
pub trait HasVault: Send {
    /// Borrow the `wasi:vault` backend context.
    fn vault_ctx(&mut self) -> &mut dyn WasiVaultCtx;
}

impl<B: HasVault + Send + 'static> WasiVaultView for omnia::StoreCtx<B> {
    fn vault(&mut self) -> WasiVaultCtxView<'_> {
        WasiVaultCtxView {
            ctx: self.backends.vault_ctx(),
            table: &mut self.base.table,
        }
    }
}

/// Generates the bundle's [`HasVault`] impl for a `runtime!` deployment.
#[macro_export]
macro_rules! omnia_wasi_view {
    ($bundle:ty, $field_name:ident) => {
        impl $crate::HasVault for $bundle {
            fn vault_ctx(&mut self) -> &mut dyn $crate::WasiVaultCtx {
                &mut self.$field_name
            }
        }
    };
}
