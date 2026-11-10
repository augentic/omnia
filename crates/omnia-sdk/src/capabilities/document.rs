//! JSON document store capability.

use std::future::Future;

use anyhow::Result;
use serde::de::DeserializeOwned;

use crate::docstore::{Document, QueryOptions, QueryResult};

/// Document storage (`wasi:docstore`).
///
/// [`WasiProvider`](crate::WasiProvider) delegates to `wasi:docstore` via
/// `omnia-wasi-docstore`. Typed reads of a document body are
/// [`DocumentStoreExt`].
#[omnia_guest_macros::capability]
pub trait DocumentStore: Send + Sync {
    /// Fetch a document by id.
    fn get(&self, store: &str, id: &str) -> impl Future<Output = Result<Option<Document>>> + Send;

    /// Insert a new document (fails if the id already exists).
    fn insert(&self, store: &str, doc: &Document) -> impl Future<Output = Result<()>> + Send;

    /// Upsert a document by id.
    fn put(&self, store: &str, doc: &Document) -> impl Future<Output = Result<()>> + Send;

    /// Delete a document by id. Returns whether a document was removed.
    fn delete(&self, store: &str, id: &str) -> impl Future<Output = Result<bool>> + Send;

    /// Query documents in a collection.
    fn query(
        &self, store: &str, options: QueryOptions,
    ) -> impl Future<Output = Result<QueryResult>> + Send;
}

#[cfg(all(target_arch = "wasm32", target_os = "wasi"))]
impl DocumentStore for crate::WasiProvider {
    async fn get(&self, store: &str, id: &str) -> Result<Option<Document>> {
        omnia_wasi_docstore::store::get(store, id).await
    }

    async fn insert(&self, store: &str, doc: &Document) -> Result<()> {
        omnia_wasi_docstore::store::insert(store, doc).await
    }

    async fn put(&self, store: &str, doc: &Document) -> Result<()> {
        omnia_wasi_docstore::store::put(store, doc).await
    }

    async fn delete(&self, store: &str, id: &str) -> Result<bool> {
        omnia_wasi_docstore::store::delete(store, id).await
    }

    async fn query(&self, store: &str, options: QueryOptions) -> Result<QueryResult> {
        omnia_wasi_docstore::store::query(store, options).await
    }
}

/// Typed reads over any [`DocumentStore`].
///
/// Blanket-implemented for every [`DocumentStore`]; never implemented by hand.
pub trait DocumentStoreExt: DocumentStore {
    /// Fetch a document by id and decode its JSON body as `T`.
    fn get_as<T: DeserializeOwned>(
        &self, store: &str, id: &str,
    ) -> impl Future<Output = Result<Option<T>>> + Send {
        async move { self.get(store, id).await?.map(|document| document.decode()).transpose() }
    }
}

impl<P: DocumentStore + ?Sized> DocumentStoreExt for P {}
