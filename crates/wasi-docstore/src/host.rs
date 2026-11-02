//! # WASI `DocStore` Service

mod resource;
mod store_impl;
mod types_impl;

mod generated {
    #![allow(missing_docs, reason = "wasmtime bindgen output")]

    pub use self::omnia::docstore::types::Error;
    pub use super::FilterProxy;

    wasmtime::component::bindgen!({
        world: "imports",
        path: "wit",
        imports: {
            default: store | tracing | trappable,
        },
        with: {
            "omnia:docstore/types.filter": FilterProxy,
        },
        trappable_error_type: {
            "omnia:docstore/types.error" => Error,
        },
    });
}

use std::fmt::Debug;

pub use omnia_backend::docstore::{
    ComparisonOp, DocStoreClient, DocStoreDefault, Document, Filter, QueryOptions, QueryResult,
    ScalarValue, SortField,
};
pub use omnia_core::FutureResult;
use omnia_core::{Host, Server, StoreView};
use wasmtime::component::{HasData, Linker};

pub use self::generated::omnia::docstore::types::Error;
use self::generated::omnia::docstore::{store, types};
pub use self::resource::*;

/// Result type for docstore operations.
pub type Result<T> = std::result::Result<T, Error>;

/// Host-side service for `omnia:docstore`.
#[derive(Debug)]
pub struct WasiDocStore;

impl HasData for WasiDocStore {
    type Data<'a> = WasiDocStoreView<'a>;
}

impl<T> Host<T> for WasiDocStore
where
    T: StoreView<Self> + 'static,
{
    fn add_to_linker(linker: &mut Linker<T>) -> anyhow::Result<()> {
        types::add_to_linker::<_, Self>(linker, T::view)?;
        Ok(store::add_to_linker::<_, Self>(linker, T::view)?)
    }
}

impl<B> Server<B> for WasiDocStore {}

omnia_core::host_error!(Error, Other);
omnia_core::wasi_view!(DocStore);
