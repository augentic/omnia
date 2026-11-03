//! In-memory `DocumentStore` over `Direct<DocStoreDefault>`.

use std::future::Future;

use anyhow::Result;
use omnia_backend::docstore::DocStoreDefault;
use omnia_native::Direct;
use omnia_sdk::DocumentStore;
use omnia_sdk::docstore::{Document, QueryOptions, QueryResult};

/// In-memory documents with the production default's filter, sort, and
/// pagination semantics; clones share one store.
///
/// Wraps [`Direct<DocStoreDefault>`], so a query behaves
/// exactly as it would through the runtime's in-memory backend.
///
/// ```
/// use omnia_sdk::DocumentStore as _;
/// use omnia_sdk::docstore::{Document, Filter, QueryOptions};
/// use omnia_test::guest::MemoryDocs;
///
/// # tokio::runtime::Runtime::new().unwrap().block_on(async {
/// let docs = MemoryDocs::default();
/// for (id, age) in [("a", 17), ("b", 42)] {
///     let doc = Document {
///         id: id.into(),
///         data: format!(r#"{{"age":{age}}}"#).into_bytes(),
///     };
///     docs.put("people", &doc).await.unwrap();
/// }
/// let options = QueryOptions {
///     filter: Some(Filter::gte("age", 18)),
///     ..QueryOptions::default()
/// };
/// let adults = docs.query("people", options).await.unwrap();
/// assert_eq!(adults.documents[0].id, "b");
/// # });
/// ```
#[derive(Clone, Debug, Default)]
pub struct MemoryDocs {
    store: Direct<DocStoreDefault>,
}

impl MemoryDocs {
    /// The wrapped host backend, for scenarios that also run the runtime.
    #[must_use]
    pub const fn backend(&self) -> &DocStoreDefault {
        &self.store.client
    }

    /// Every document in `store`, following continuations to the end.
    ///
    /// # Errors
    ///
    /// Returns the backend's error.
    pub async fn documents(&self, store: &str) -> Result<Vec<Document>> {
        let mut all = Vec::new();
        let mut continuation = None;
        loop {
            let page = self
                .store
                .query(
                    store,
                    QueryOptions {
                        continuation,
                        ..QueryOptions::default()
                    },
                )
                .await?;
            all.extend(page.documents);
            continuation = page.continuation;
            if continuation.is_none() {
                return Ok(all);
            }
        }
    }
}

impl DocumentStore for MemoryDocs {
    fn get(&self, store: &str, id: &str) -> impl Future<Output = Result<Option<Document>>> + Send {
        self.store.get(store, id)
    }

    fn insert(&self, store: &str, doc: &Document) -> impl Future<Output = Result<()>> + Send {
        self.store.insert(store, doc)
    }

    fn put(&self, store: &str, doc: &Document) -> impl Future<Output = Result<()>> + Send {
        self.store.put(store, doc)
    }

    fn delete(&self, store: &str, id: &str) -> impl Future<Output = Result<bool>> + Send {
        self.store.delete(store, id)
    }

    fn query(
        &self, store: &str, options: QueryOptions,
    ) -> impl Future<Output = Result<QueryResult>> + Send {
        self.store.query(store, options)
    }
}
