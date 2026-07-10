//! # Host implementation for WASI Identity Service
//!
//! This module implements the host-side logic for the WASI Identity service.

mod credentials_impl;
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

pub use omnia::FutureResult;
use omnia::{Host, Server};
use wasmtime::component::{HasData, Linker};

pub use self::default_impl::IdentityDefault;
use self::generated::omnia::identity::credentials;
pub use self::resource::*;
pub use self::stub_impl::IdentityStub;
use crate::host::generated::Error;

/// Result type for identity operations.
pub type Result<T> = anyhow::Result<T, Error>;

/// Host-side service for `wasi:identity`.
#[derive(Debug)]
pub struct WasiIdentity;

impl HasData for WasiIdentity {
    type Data<'a> = WasiIdentityCtxView<'a>;
}

impl<T> Host<T> for WasiIdentity
where
    T: WasiIdentityView + 'static,
{
    fn add_to_linker(linker: &mut Linker<T>) -> anyhow::Result<()> {
        Ok(credentials::add_to_linker::<_, Self>(linker, T::identity)?)
    }
}

impl<B> Server<B> for WasiIdentity {}

/// A trait which provides internal WASI Identity context.
///
/// This is implemented by the resource-specific provider of Identity
/// functionality.
pub trait WasiIdentityCtx: Debug + Send + Sync + 'static {
    /// Get the identity for the specified name.
    fn get_identity(&self, name: String) -> FutureResult<Arc<dyn Identity>>;
}

omnia::host_error!(Error, InternalFailure);
omnia::wasi_view!(Identity);
