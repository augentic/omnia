//! In-memory default implementation of [`DocStoreClient`].

use std::fmt::Debug;

use anyhow::Result;
use futures::FutureExt as _;
use omnia_backend::docstore::filter::MemoryStore;
use omnia_backend::docstore::{DocStoreClient, Document, QueryOptions, QueryResult};
use omnia_core::{Connect, FutureResult, NoOptions};
use tracing::instrument;

/// Default in-memory implementation of [`DocStoreClient`].
#[derive(Clone, Default)]
pub struct DocStoreDefault {
    store: MemoryStore,
}

impl Debug for DocStoreDefault {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DocStoreDefault").finish_non_exhaustive()
    }
}

impl Connect for DocStoreDefault {
    type ConnectOptions = NoOptions;

    #[instrument]
    async fn connect_with(options: Self::ConnectOptions) -> Result<Self> {
        let _ = options;
        tracing::debug!("initializing in-memory document store");
        Ok(Self::default())
    }
}

impl DocStoreClient for DocStoreDefault {
    fn get(&self, collection: String, id: String) -> FutureResult<Option<Document>> {
        let store = self.store.clone();
        async move { store.get(&collection, &id) }.boxed()
    }

    fn insert(&self, collection: String, doc: Document) -> FutureResult<()> {
        let store = self.store.clone();
        async move { store.insert(&collection, &doc) }.boxed()
    }

    fn put(&self, collection: String, doc: Document) -> FutureResult<()> {
        let store = self.store.clone();
        async move { store.put(&collection, &doc) }.boxed()
    }

    fn delete(&self, collection: String, id: String) -> FutureResult<bool> {
        let store = self.store.clone();
        async move { Ok(store.delete(&collection, &id)) }.boxed()
    }

    fn query(&self, collection: String, options: QueryOptions) -> FutureResult<QueryResult> {
        let store = self.store.clone();
        async move { store.query(&collection, &options) }.boxed()
    }
}
