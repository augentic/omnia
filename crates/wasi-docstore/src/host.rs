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
use wasmtime::component::{Linker, ResourceTableError};

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
///
/// A backend picks the kind of a failure by returning an [`Error`], and any
/// other error reaches the guest as [`Error::Other`].
pub trait DocStoreClient: Debug + Send + Sync + 'static {
    /// Point read by primary id.
    fn get(&self, collection: String, id: String) -> FutureResult<Option<Document>>;

    /// Insert `doc` unless its id is already stored.
    ///
    /// Returns `false`, and leaves the stored document untouched, when the id
    /// is already stored.
    fn insert(&self, collection: String, doc: Document) -> FutureResult<bool>;

    /// Upsert by id.
    fn put(&self, collection: String, doc: Document) -> FutureResult<()>;

    /// Delete by id; `Ok(true)` if a document was removed.
    fn delete(&self, collection: String, id: String) -> FutureResult<bool>;

    /// One page of the documents in `collection` that `filter` and `options` select.
    ///
    /// A backend serves the portable subset the
    /// [document store reference](https://github.com/augentic/omnia/blob/main/docs/reference/docstore.md#query-contract)
    /// lists. It fails any other feature it cannot serve with
    /// [`Error::Unsupported`], and never ignores one.
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

// a backend picks the kind by returning an `Error`; any other failure is
// `other`, with its full context chain
impl From<anyhow::Error> for Error {
    fn from(err: anyhow::Error) -> Self {
        err.downcast::<Self>().unwrap_or_else(|err| Self::Other(format!("{err:#}")))
    }
}

impl From<ResourceTableError> for Error {
    fn from(err: ResourceTableError) -> Self {
        Self::Other(err.to_string())
    }
}
