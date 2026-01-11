//! # Host implementation for WASI Vault Service
//!
//! This module implements the host-side logic for the WASI Vault service.

pub mod default_impl;
mod resource;
mod vault_impl;

mod generated {
    #![allow(clippy::trait_duplication_in_bounds)]

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

use wasmtime::component::{HasData, Linker};
use wasmtime_wasi::ResourceTable;
pub use yetti::FutureResult;
use yetti::{Host, Server, State};

use self::generated::wasi::vault::vault;
pub use crate::host::default_impl::VaultDefault;
pub use crate::host::resource::*;

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
        vault::add_to_linker::<_, Self>(linker, T::vault)
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
    fn open_locker(&self, identifier: String) -> FutureResult<Arc<dyn Locker>>;
}

#[macro_export]
macro_rules! wasi_view {
    ($store_ctx:ty, $field_name:ident) => {
        impl yetti_wasi_vault::WasiVaultView for $store_ctx {
            fn vault(&mut self) -> yetti_wasi_vault::WasiVaultCtxView<'_> {
                yetti_wasi_vault::WasiVaultCtxView {
                    ctx: &mut self.$field_name,
                    table: &mut self.table,
                }
            }
        }
    };
}
