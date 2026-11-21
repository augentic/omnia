#![doc = include_str!("../README.md")]

//! # WASI Identity Service
//!
//! This module implements a runtime service for `omnia:identity`.

mod credentials_impl;
#[cfg(feature = "oauth")]
mod default_impl;
mod resource;
mod stub_impl;

mod generated {
    pub use omnia_wasi_types::Error;

    pub use crate::resource::IdentityProxy;

    wasmtime::component::bindgen!({
        world: "imports",
        path: "wit",
        imports: {
            default: store | tracing | trappable,
        },
        with: {
            "omnia:identity/credentials.identity": IdentityProxy,
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

#[cfg(feature = "oauth")]
pub use self::default_impl::IdentityDefault;
pub use self::generated::Error;
use self::generated::omnia::identity::{credentials, types};
pub use self::resource::*;
pub use self::stub_impl::{IdentityStub, StubOptions};

/// Result type for identity operations.
pub type Result<T> = std::result::Result<T, Error>;

/// Host-side service for `omnia:identity`.
#[derive(Debug, HostCtx)]
#[host_ctx(client = IdentityClient)]
pub struct WasiIdentity;

impl<B: Provides<Self>> Host<B> for WasiIdentity {
    fn add_to_linker(linker: &mut Linker<StoreCtx<B>>) -> anyhow::Result<()> {
        omnia_wasi_types::types::add_to_linker::<_, Self>(linker, StoreCtx::view::<Self>)?;
        types::add_to_linker::<_, Self>(linker, StoreCtx::view::<Self>)?;
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
    /// [`Error::NotFound`].
    fn get_identity(&self, name: String) -> FutureResult<Arc<dyn Identity>>;
}

impl types::Host for HostView<'_, dyn IdentityClient> {}
