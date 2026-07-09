//! # WASI `DocStore` Service

mod default_impl;
mod resource;
mod store_impl;
mod types_impl;

mod generated {
    #![allow(missing_docs)]

    pub use self::wasi::docstore::types::Error;
    pub use super::FilterProxy;

    wasmtime::component::bindgen!({
        world: "imports",
        path: "wit",
        imports: {
            default: store | tracing | trappable,
        },
        with: {
            "wasi:docstore/types.filter": FilterProxy,
        },
        trappable_error_type: {
            "wasi:docstore/types.error" => Error,
        },
    });
}

use std::fmt::Debug;

pub use omnia::FutureResult;
use omnia::{Host, Server};
use wasmtime::component::{HasData, Linker, ResourceTableError};

pub use self::default_impl::DocStoreDefault;
pub use self::generated::wasi::docstore::types::{
    ComparisonOp, Document, Error, QueryResult, ScalarValue, SortField,
};
use self::generated::wasi::docstore::{store, types};
pub use self::resource::*;

/// Result type for docstore operations.
pub type Result<T> = anyhow::Result<T, Error>;

/// Host-side service for `wasi:docstore`.
#[derive(Debug)]
pub struct WasiDocStore;

impl HasData for WasiDocStore {
    type Data<'a> = WasiDocStoreCtxView<'a>;
}

impl<T> Host<T> for WasiDocStore
where
    T: WasiDocStoreView + 'static,
{
    fn add_to_linker(linker: &mut Linker<T>) -> anyhow::Result<()> {
        types::add_to_linker::<_, Self>(linker, T::docstore)?;
        Ok(store::add_to_linker::<_, Self>(linker, T::docstore)?)
    }
}

impl<B> Server<B> for WasiDocStore {}

/// A trait which provides internal WASI `DocStore` context.
///
/// This is implemented by the resource-specific provider of `DocStore`
/// functionality. For example, an embedded `PoloDB` file, or Azure Table
/// Storage.
pub trait WasiDocStoreCtx: Debug + Send + Sync + 'static {
    /// Point read by primary id.
    fn get(&self, collection: String, id: String) -> FutureResult<Option<Document>>;

    /// Insert if absent.
    fn insert(&self, collection: String, doc: Document) -> FutureResult<()>;

    /// Upsert by id.
    fn put(&self, collection: String, doc: Document) -> FutureResult<()>;

    /// Delete by id; `Ok(true)` if a document was removed.
    fn delete(&self, collection: String, id: String) -> FutureResult<bool>;

    /// Filtered listing with sort and pagination.
    fn query(
        &self, collection: String, filter: Option<FilterTree>, options: QueryOpts,
    ) -> FutureResult<QueryResult>;
}

/// Host-side query options (WIT `query-options` without the filter resource).
#[derive(Debug, Clone, Default)]
pub struct QueryOpts {
    /// Sort fields.
    pub order_by: Vec<SortField>,
    /// Max documents.
    pub limit: Option<u32>,
    /// Skip count (when no continuation token).
    pub offset: Option<u32>,
    /// Opaque continuation.
    pub continuation: Option<String>,
}

impl From<anyhow::Error> for Error {
    fn from(err: anyhow::Error) -> Self {
        // `:#` keeps the full context chain from backend errors.
        Self::Other(format!("{err:#}"))
    }
}

impl From<ResourceTableError> for Error {
    fn from(err: ResourceTableError) -> Self {
        Self::Other(err.to_string())
    }
}

omnia::wasi_view!(DocStore);
