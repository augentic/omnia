//! # Host implementation for WASI Identity Service
//!
//! This module implements the host-side logic for the WASI Identity service.

mod credentials_impl;
mod default_impl;
mod resource;
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
use wasmtime::component::{HasData, Linker, ResourceTable, ResourceTableError};

pub use self::default_impl::IdentityDefault;
use self::generated::omnia::identity::credentials;
pub use self::resource::*;
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

/// A trait which provides internal WASI Identity state.
///
/// This is implemented by the `T` in `Linker<T>` — a single type shared across
/// all WASI components for the runtime build.
pub trait WasiIdentityView: Send {
    /// Return a [`WasiIdentityCtxView`] from mutable reference to self.
    fn identity(&mut self) -> WasiIdentityCtxView<'_>;
}

/// View into [`WasiIdentityCtx`] implementation and [`ResourceTable`].
pub struct WasiIdentityCtxView<'a> {
    /// Mutable reference to the WASI Identity context.
    pub ctx: &'a mut dyn WasiIdentityCtx,

    /// Mutable reference to table used to manage resources.
    pub table: &'a mut ResourceTable,
}

/// A trait which provides internal WASI Identity context.
///
/// This is implemented by the resource-specific provider of Identity
/// functionality.
pub trait WasiIdentityCtx: Debug + Send + Sync + 'static {
    /// Get the identity for the specified name.
    fn get_identity(&self, name: String) -> FutureResult<Arc<dyn Identity>>;
}

/// `anyhow::Error` to `Error` mapping
impl From<anyhow::Error> for Error {
    fn from(err: anyhow::Error) -> Self {
        Self::InternalFailure(err.to_string())
    }
}

/// `ResourceTableError` to `Error` mapping
impl From<ResourceTableError> for Error {
    fn from(err: ResourceTableError) -> Self {
        Self::InternalFailure(err.to_string())
    }
}

/// A backend bundle that can yield the `wasi:identity` backend for a store.
///
/// The blanket [`WasiIdentityView`] impl below turns this accessor into the
/// linker-facing view on `omnia::StoreCtx<B>`; the `runtime!` macro generates
/// the bundle-side impl via [`omnia_wasi_view!`].
pub trait HasIdentity: Send {
    /// Borrow the `wasi:identity` backend context.
    fn identity_ctx(&mut self) -> &mut dyn WasiIdentityCtx;
}

impl<B: HasIdentity + Send + 'static> WasiIdentityView for omnia::StoreCtx<B> {
    fn identity(&mut self) -> WasiIdentityCtxView<'_> {
        WasiIdentityCtxView {
            ctx: self.backends.identity_ctx(),
            table: &mut self.base.table,
        }
    }
}

/// Generates the bundle's [`HasIdentity`] impl for a `runtime!` deployment.
#[macro_export]
macro_rules! omnia_wasi_view {
    ($bundle:ty, $field_name:ident) => {
        impl $crate::HasIdentity for $bundle {
            fn identity_ctx(&mut self) -> &mut dyn $crate::WasiIdentityCtx {
                &mut self.$field_name
            }
        }
    };
}
