//! # WASI Vault Service

mod default_impl;
mod locker_impl;
mod resource;
mod store_impl;
mod types_impl;

mod generated {
    pub use self::omnia::vault::types::Error;
    pub use crate::host::resource::LockerProxy;

    wasmtime::component::bindgen!({
        world: "imports",
        path: "wit",
        imports: {
            default: store | tracing | trappable,
        },
        with: {
            "omnia:vault/locker.locker": LockerProxy,
        },
        trappable_error_type: {
            "omnia:vault/types.error" => Error,
        },
    });
}

use std::fmt::Debug;
use std::sync::Arc;

pub use omnia_core::FutureResult;
use omnia_core::{Host, HostCtx, Provides, StoreCtx};
use wasmtime::component::{Linker, ResourceTableError};

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

// a backend picks the kind by returning an `Error`; any other failure is
// `other`, with its full context chain
impl From<anyhow::Error> for Error {
    fn from(err: anyhow::Error) -> Self {
        err.downcast::<Self>().unwrap_or_else(|err| Self::Other(format!("{err:#}")))
    }
}

impl From<ResourceTableError> for Error {
    fn from(err: ResourceTableError) -> Self {
        Self::Other(err.to_string())
    }
}
