#![doc = include_str!("../README.md")]

//! # WASI Vault
//!
//! This module implements a runtime service for `omnia:vault`.

mod default_impl;
mod locker_impl;
mod resource;
mod store_impl;

mod generated {
    pub use omnia_wasi_types::Error;

    pub use crate::resource::LockerProxy;

    wasmtime::component::bindgen!({
        world: "imports",
        path: "wit",
        imports: {
            default: store | tracing | trappable,
        },
        with: {
            "omnia:vault/locker.locker": LockerProxy,
            "omnia:types/types": omnia_wasi_types::types,
        },
        trappable_error_type: {
            "omnia:types/types.error" => Error,
        },
    });
}

use std::fmt::Debug;
use std::sync::Arc;

pub use omnia_core::FutureResult;
use omnia_core::{Host, HostCtx, HostView, Provides, StoreCtx};
use wasmtime::component::Linker;

pub use self::default_impl::{ConnectOptions, VaultDefault};
pub use self::generated::Error;
use self::generated::omnia::vault::{locker, store, types};
pub use self::resource::*;

/// Result type for vault operations.
pub type Result<T> = std::result::Result<T, Error>;

/// Host-side service for `omnia:vault`.
#[derive(Debug, HostCtx)]
#[host_ctx(client = VaultClient)]
pub struct WasiVault;

impl<B: Provides<Self>> Host<B> for WasiVault {
    fn add_to_linker(linker: &mut Linker<StoreCtx<B>>) -> anyhow::Result<()> {
        omnia_wasi_types::types::add_to_linker::<_, Self>(linker, StoreCtx::view::<Self>)?;
        types::add_to_linker::<_, Self>(linker, StoreCtx::view::<Self>)?;
        locker::add_to_linker::<_, Self>(linker, StoreCtx::view::<Self>)?;
        Ok(store::add_to_linker::<_, Self>(linker, StoreCtx::view::<Self>)?)
    }
}

/// Service contract for `omnia:vault` backends.
///
/// This is implemented by the resource-specific provider of Vault
/// functionality.
pub trait VaultClient: Debug + Send + Sync + 'static {
    /// Open the locker the deployment declares under `identifier`.
    ///
    /// An identifier the deployment does not declare fails with
    /// [`Error::NotFound`].
    fn open_locker(&self, identifier: String) -> FutureResult<Arc<dyn Locker>>;
}

impl types::Host for HostView<'_, dyn VaultClient> {}
