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

#[cfg(feature = "oauth")]
pub use default_impl::{ConnectOptions, IdentityDefault};
pub use omnia_backend::identity::{AccessToken, Identity, IdentityClient};
pub use omnia_core::FutureResult;
use omnia_core::{Host, HostCtx, Server, StoreView};
pub use stub_impl::IdentityStub;
use wasmtime::component::Linker;

use self::generated::omnia::identity::credentials;
pub use self::resource::*;
use crate::host::generated::Error;

/// Result type for identity operations.
pub type Result<T> = std::result::Result<T, Error>;

/// Host-side service for `wasi:identity`.
#[derive(Debug, HostCtx)]
pub struct WasiIdentity;

impl<T> Host<T> for WasiIdentity
where
    T: StoreView<Self> + 'static,
{
    fn add_to_linker(linker: &mut Linker<T>) -> anyhow::Result<()> {
        Ok(credentials::add_to_linker::<_, Self>(linker, T::view)?)
    }
}

impl<B> Server<B> for WasiIdentity {}

omnia_core::host_error!(Error, InternalFailure);
