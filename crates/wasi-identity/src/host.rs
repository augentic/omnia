//! # Host implementation for WASI Identity Service
//!
//! This module implements the host-side logic for the WASI Identity service.

mod credentials_impl;
#[cfg(feature = "oauth")]
mod default_impl;
mod resource;
mod stub_impl;
mod types_impl;

mod generated {
    pub use self::omnia::identity::types::Error;
    pub use crate::host::resource::IdentityProxy;

    wasmtime::component::bindgen!({
        world: "imports",
        path: "wit",
        imports: {
            default: store | tracing | trappable,
        },
        with: {
            "omnia:identity/credentials.identity": IdentityProxy,
        },
        trappable_error_type: {
            "omnia:identity/types.error" => Error,
        },
    });
}

use std::fmt::Debug;
use std::sync::Arc;

pub use omnia_core::FutureResult;
use omnia_core::{Host, HostCtx, Provides, StoreCtx};
use wasmtime::component::{Linker, ResourceTableError};

#[cfg(feature = "oauth")]
pub use self::default_impl::IdentityDefault;
pub use self::generated::Error;
use self::generated::omnia::identity::credentials;
pub use self::resource::*;
pub use self::stub_impl::{IdentityStub, StubOptions};

/// Result type for identity operations.
pub type Result<T> = std::result::Result<T, Error>;

/// Host-side service for `wasi:identity`.
#[derive(Debug, HostCtx)]
#[host_ctx(client = IdentityClient)]
pub struct WasiIdentity;

impl<B: Provides<Self>> Host<B> for WasiIdentity {
    fn add_to_linker(linker: &mut Linker<StoreCtx<B>>) -> anyhow::Result<()> {
        Ok(credentials::add_to_linker::<_, Self>(linker, StoreCtx::view::<Self>)?)
    }
}

/// Service contract for `omnia:identity` backends.
///
/// This is implemented by the resource-specific provider of Identity
/// functionality.
pub trait IdentityClient: Debug + Send + Sync + 'static {
    /// Get the identity the deployment declares under `name`.
    ///
    /// A name the deployment does not declare fails with
    /// [`Error::NoSuchIdentity`].
    fn get_identity(&self, name: String) -> FutureResult<Arc<dyn Identity>>;
}

// a backend picks the kind by returning an `Error`; any other failure is
// `internal-failure`, with its full context chain
impl From<anyhow::Error> for Error {
    fn from(err: anyhow::Error) -> Self {
        err.downcast::<Self>().unwrap_or_else(|err| Self::InternalFailure(format!("{err:#}")))
    }
}

impl From<ResourceTableError> for Error {
    fn from(err: ResourceTableError) -> Self {
        Self::InternalFailure(err.to_string())
    }
}
