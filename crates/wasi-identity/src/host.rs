//! # Host implementation for WASI Identity Service
//!
//! This module implements the host-side logic for the WASI Identity service.

mod credentials_impl;
mod default_impl;
mod resource;
mod types_impl;

mod generated {
    #![allow(clippy::trait_duplication_in_bounds)]

    pub use self::wasi::identity::types::Error;
    pub use crate::host::resource::IdentityProxy;

    wasmtime::component::bindgen!({
        world: "identity",
        path: "wit",
        imports: {
            default: store | tracing | trappable,
        },
        with: {
            "wasi:identity/credentials.identity": IdentityProxy,
        },
        trappable_error_type: {
            "wasi:identity/types.error" => Error,
        },
    });
}

use std::fmt::Debug;
use std::sync::Arc;

use wasmtime::component::{HasData, Linker};
use wasmtime_wasi::ResourceTable;
pub use yetti::FutureResult;
use yetti::{Host, Server, State};

pub use self::default_impl::IdentityDefault;
use self::generated::wasi::identity::credentials;
pub use self::resource::*;

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
        credentials::add_to_linker::<_, Self>(linker, T::identity)
    }
}

impl<S> Server<S> for WasiIdentity where S: State {}

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
    fn get_identity(&self, name: String) -> FutureResult<Arc<dyn Identity>>;
}

#[macro_export]
macro_rules! wasi_view {
    ($store_ctx:ty, $field_name:ident) => {
        impl yetti_wasi_identity::WasiIdentityView for $store_ctx {
            fn identity(&mut self) -> yetti_wasi_identity::WasiIdentityCtxView<'_> {
                yetti_wasi_identity::WasiIdentityCtxView {
                    ctx: &mut self.$field_name,
                    table: &mut self.table,
                }
            }
        }
    };
}
