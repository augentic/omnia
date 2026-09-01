//! # WASI Key-Value Service

mod atomics_impl;
mod batch_impl;
mod default_impl;
mod resource;
mod store_impl;

mod generated {
    pub use self::wasi::keyvalue::atomics::CasError;
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
            "wasi:keyvalue/atomics.cas-error" => CasError,
        },

    });
}

use std::fmt::Debug;
use std::sync::Arc;

pub use omnia_core::FutureResult;
use omnia_core::{Host, Server, StoreView};
use wasmtime::component::{HasData, Linker};

pub use self::default_impl::KeyValueDefault;
use self::generated::wasi::keyvalue::store::Error;
use self::generated::wasi::keyvalue::{atomics, batch, store};
pub use self::resource::*;

/// Result type for key-value operations.
pub type Result<T> = std::result::Result<T, Error>;

/// Host-side service for `wasi:keyvalue`.
#[derive(Debug)]
pub struct WasiKeyValue;

impl HasData for WasiKeyValue {
    type Data<'a> = WasiKeyValueCtxView<'a>;
}

impl<T> Host<T> for WasiKeyValue
where
    T: StoreView<Self> + 'static,
{
    fn add_to_linker(linker: &mut Linker<T>) -> anyhow::Result<()> {
        store::add_to_linker::<_, Self>(linker, T::view)?;
        atomics::add_to_linker::<_, Self>(linker, T::view)?;
        Ok(batch::add_to_linker::<_, Self>(linker, T::view)?)
    }
}

impl<B> Server<B> for WasiKeyValue {}

/// A trait which provides internal WASI Key-Value context.
///
/// This is implemented by the resource-specific provider of Key-Value
/// functionality. For example, an in-memory store, or a Redis-backed store.
pub trait WasiKeyValueCtx: Debug + Send + Sync + 'static {
    /// Open a bucket.
    fn open_bucket(&self, identifier: String) -> FutureResult<Arc<dyn Bucket>>;
}

omnia_core::host_error!(Error, Other);
omnia_core::wasi_view!(KeyValue);
