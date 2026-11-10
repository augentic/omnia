//! # WASI `DocStore` Service

mod default_impl;
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

pub use omnia_core::FutureResult;
use omnia_core::{Host, HostCtx, Provides, StoreCtx};
use wasmtime::component::Linker;

pub use self::default_impl::DocStoreDefault;
pub use self::generated::omnia::docstore::types::{
    ComparisonOp, Document, Error, QueryResult, ScalarValue, SortField,
};
use self::generated::omnia::docstore::{store, types};
pub use self::resource::*;

/// Result type for docstore operations.
pub type Result<T> = std::result::Result<T, Error>;

/// Host-side service for `omnia:docstore`.
#[derive(Debug, HostCtx)]
#[host_ctx(client = DocStoreClient)]
pub struct WasiDocStore;

impl<B: Provides<Self>> Host<B> for WasiDocStore {
    fn add_to_linker(linker: &mut Linker<StoreCtx<B>>) -> anyhow::Result<()> {
        types::add_to_linker::<_, Self>(linker, StoreCtx::view::<Self>)?;
        Ok(store::add_to_linker::<_, Self>(linker, StoreCtx::view::<Self>)?)
    }
}

/// Service contract for `omnia:docstore` backends.
///
/// This is implemented by the resource-specific provider of `DocStore`
/// functionality. For example, the in-memory default, or Azure Table
/// Storage.
pub trait DocStoreClient: Debug + Send + Sync + 'static {
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

omnia_core::host_error!(Error, Other);
