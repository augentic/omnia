//! JSON document store capability.

use std::future::Future;

use anyhow::Result;

use crate::document_store::{Document, QueryOptions, QueryResult};

/// JSON document storage (WASI JSON DB).
///
/// Default WASM implementations delegate to `wasi:jsondb` via `omnia-wasi-jsondb`.
pub trait DocumentStore: Send + Sync {
    /// Fetch a document by id.
    #[cfg(not(target_arch = "wasm32"))]
    fn get(&self, store: &str, id: &str) -> impl Future<Output = Result<Option<Document>>> + Send;

    /// Insert a new document (fails if the id already exists).
    #[cfg(not(target_arch = "wasm32"))]
    fn insert(&self, store: &str, doc: &Document) -> impl Future<Output = Result<()>> + Send;

    /// Upsert a document by id.
    #[cfg(not(target_arch = "wasm32"))]
    fn put(&self, store: &str, doc: &Document) -> impl Future<Output = Result<()>> + Send;

    /// Delete a document by id. Returns whether a document was removed.
    #[cfg(not(target_arch = "wasm32"))]
    fn delete(&self, store: &str, id: &str) -> impl Future<Output = Result<bool>> + Send;

    /// Query documents in a collection.
    #[cfg(not(target_arch = "wasm32"))]
    fn query(
        &self, store: &str, options: QueryOptions,
    ) -> impl Future<Output = Result<QueryResult>> + Send;

    /// Fetch a document by id.
    #[cfg(target_arch = "wasm32")]
    fn get(&self, store: &str, id: &str) -> impl Future<Output = Result<Option<Document>>> + Send {
        async move { omnia_wasi_jsondb::store::get(store, id).await }
    }

    /// Insert a new document (fails if the id already exists).
    #[cfg(target_arch = "wasm32")]
    fn insert(&self, store: &str, doc: &Document) -> impl Future<Output = Result<()>> + Send {
        async move { omnia_wasi_jsondb::store::insert(store, doc).await }
    }

    /// Upsert a document by id.
    #[cfg(target_arch = "wasm32")]
    fn put(&self, store: &str, doc: &Document) -> impl Future<Output = Result<()>> + Send {
        async move { omnia_wasi_jsondb::store::put(store, doc).await }
    }

    /// Delete a document by id. Returns whether a document was removed.
    #[cfg(target_arch = "wasm32")]
    fn delete(&self, store: &str, id: &str) -> impl Future<Output = Result<bool>> + Send {
        async move { omnia_wasi_jsondb::store::delete(store, id).await }
    }

    /// Query documents in a collection.
    #[cfg(target_arch = "wasm32")]
    fn query(
        &self, store: &str, options: QueryOptions,
    ) -> impl Future<Output = Result<QueryResult>> + Send {
        async move { omnia_wasi_jsondb::store::query(store, options).await }
    }
}
