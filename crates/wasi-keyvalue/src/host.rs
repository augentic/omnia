//! # WASI Key-Value Service

mod atomics_impl;
mod batch_impl;
mod default_impl;
mod resource;
mod store_impl;

mod generated {
    pub use self::wasi::keyvalue::store::Error;
    pub use super::{BucketProxy, Cas};

    wasmtime::component::bindgen!({
        world: "imports",
        path: "wit",
        imports: {
            default: store | tracing | trappable,
        },
        with: {
            "wasi:keyvalue/store.bucket": BucketProxy,
            "wasi:keyvalue/atomics.cas": Cas,
        },
        trappable_error_type: {
            "wasi:keyvalue/store.error" => Error,
        },

    });
}

use std::fmt::Debug;
use std::sync::Arc;

pub use omnia::FutureResult;
use omnia::{Host, Server};
use wasmtime::component::{HasData, Linker, ResourceTable, ResourceTableError};

pub use self::default_impl::KeyValueDefault;
use self::generated::wasi::keyvalue::store::Error;
use self::generated::wasi::keyvalue::{atomics, batch, store};
pub use self::resource::*;

/// Result type for key-value operations.
pub type Result<T, E = Error> = anyhow::Result<T, E>;

/// Host-side service for `wasi:keyvalue`.
#[derive(Debug)]
pub struct WasiKeyValue;

impl HasData for WasiKeyValue {
    type Data<'a> = WasiKeyValueCtxView<'a>;
}

impl<T> Host<T> for WasiKeyValue
where
    T: WasiKeyValueView + 'static,
{
    fn add_to_linker(linker: &mut Linker<T>) -> anyhow::Result<()> {
        store::add_to_linker::<_, Self>(linker, T::keyvalue)?;
        atomics::add_to_linker::<_, Self>(linker, T::keyvalue)?;
        Ok(batch::add_to_linker::<_, Self>(linker, T::keyvalue)?)
    }
}

impl<B> Server<B> for WasiKeyValue {}

/// A trait which provides internal WASI Key-Value state.
///
/// This is implemented by the `T` in `Linker<T>` — a single type shared across
/// all WASI components for the runtime build.
pub trait WasiKeyValueView: Send {
    /// Return a [`WasiKeyValueCtxView`] from mutable reference to self.
    fn keyvalue(&mut self) -> WasiKeyValueCtxView<'_>;
}

/// View into [`WasiKeyValueCtx`] implementation and [`ResourceTable`].
pub struct WasiKeyValueCtxView<'a> {
    /// Mutable reference to the WASI Key-Value context.
    pub ctx: &'a mut dyn WasiKeyValueCtx,

    /// Mutable reference to table used to manage resources.
    pub table: &'a mut ResourceTable,
}

/// A trait which provides internal WASI Key-Value context.
///
/// This is implemented by the resource-specific provider of Key-Value
/// functionality. For example, an in-memory store, or a Redis-backed store.
pub trait WasiKeyValueCtx: Debug + Send + Sync + 'static {
    /// Open a bucket.
    fn open_bucket(&self, identifier: String) -> FutureResult<Arc<dyn Bucket>>;
}

impl From<anyhow::Error> for Error {
    fn from(err: anyhow::Error) -> Self {
        Self::Other(err.to_string())
    }
}

impl From<ResourceTableError> for Error {
    fn from(err: ResourceTableError) -> Self {
        Self::Other(err.to_string())
    }
}

/// A backend bundle that can yield the `wasi:keyvalue` backend for a store.
///
/// The blanket [`WasiKeyValueView`] impl below turns this accessor into the
/// linker-facing view on `omnia::StoreCtx<B>`; the `runtime!` macro generates
/// the bundle-side impl via [`omnia_wasi_view!`].
pub trait HasKeyValue: Send {
    /// Borrow the `wasi:keyvalue` backend context.
    fn keyvalue_ctx(&mut self) -> &mut dyn WasiKeyValueCtx;
}

impl<B: HasKeyValue + Send + 'static> WasiKeyValueView for omnia::StoreCtx<B> {
    fn keyvalue(&mut self) -> WasiKeyValueCtxView<'_> {
        WasiKeyValueCtxView {
            ctx: self.backends.keyvalue_ctx(),
            table: &mut self.base.table,
        }
    }
}

/// Generates the bundle's [`HasKeyValue`] impl for a `runtime!` deployment.
#[macro_export]
macro_rules! omnia_wasi_view {
    ($bundle:ty, $field_name:ident) => {
        impl $crate::HasKeyValue for $bundle {
            fn keyvalue_ctx(&mut self) -> &mut dyn $crate::WasiKeyValueCtx {
                &mut self.$field_name
            }
        }
    };
}
