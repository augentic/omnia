#![doc = include_str!("../README.md")]

//! # WASI `DocStore`
//!
//! This module implements a runtime service for `omnia:docstore`: a JSON
//! document store with a backend-portable filter language.

mod collection_impl;
mod default_impl;
mod resource;
mod store_impl;

#[expect(missing_docs, reason = "the WIT documents no self-describing field")]
mod generated {
    pub use omnia_wasi_types::Error;

    pub use super::collection_impl::CollectionHandle;

    wasmtime::component::bindgen!({
        world: "imports",
        path: "wit",
        imports: {
            default: store | tracing | trappable,
        },
        with: {
            "omnia:docstore/collection.collection": CollectionHandle,
            "omnia:types/types": omnia_wasi_types::types,
        },
        trappable_error_type: {
            "omnia:types/types.error" => Error,
        },
    });
}

use std::fmt::Debug;

pub use omnia_core::FutureResult;
use omnia_core::{Host, HostCtx, HostView, Provides, StoreCtx};
use wasmtime::component::Linker;

pub use self::default_impl::{ConnectOptions, DocStoreDefault};
pub use self::generated::Error;
pub use self::generated::omnia::docstore::collection::{Condition, Filter, SortField, Test, Value};
use self::generated::omnia::docstore::types::Host as TypesHost;
use self::generated::omnia::docstore::{collection, store, types};
pub use self::resource::*;

/// Result type for docstore operations.
pub type Result<T> = std::result::Result<T, Error>;

/// Host-side service for `omnia:docstore`.
#[derive(Debug, HostCtx)]
#[host_ctx(client = DocStoreClient)]
pub struct WasiDocStore;

impl<B: Provides<Self>> Host<B> for WasiDocStore {
    fn add_to_linker(linker: &mut Linker<StoreCtx<B>>) -> anyhow::Result<()> {
        omnia_wasi_types::types::add_to_linker::<_, Self>(linker, StoreCtx::view::<Self>)?;
        types::add_to_linker::<_, Self>(linker, StoreCtx::view::<Self>)?;
        collection::add_to_linker::<_, Self>(linker, StoreCtx::view::<Self>)?;
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
    /// A backend serves the portable query the
    /// [document store reference](https://github.com/augentic/omnia/blob/main/docs/reference/docstore.md#query-contract)
    /// defines. It fails any other feature it cannot serve with
    /// [`Error::Unsupported`], and never ignores one.
    ///
    /// An empty [`Filter`] selects every document. The host has already
    /// refused any filter the WIT rejects, so in the one a backend is handed:
    ///
    /// - every group holds at least one [`Condition`];
    /// - the groups hold at most 100 conditions, and each [`Test::In`] 1 to
    ///   100 values;
    /// - no ordering [`Test`] is given a [`Value::Boolean`], and every
    ///   [`Value::Number`] is finite.
    fn query(
        &self, collection: String, filter: Filter, options: QueryOpts,
    ) -> FutureResult<QueryResult>;
}

/// The ordering and paging of one query, as the host hands them to a backend.
#[derive(Debug, Clone)]
pub struct QueryOpts {
    /// Sort fields.
    pub order_by: Vec<SortField>,
    /// The most documents the page holds, from 1 to 1,000.
    pub limit: u32,
    /// The cursor a previous page returned, `None` for the first page.
    pub continuation: Option<String>,
}

impl TypesHost for HostView<'_, dyn DocStoreClient> {}
