//! In-memory `DocumentStore` over `DocStoreDefault`.

use anyhow::Result;
use omnia_backend::docstore::{DocStoreClient as _, DocStoreDefault};
use omnia_sdk::DocumentStore;
use omnia_sdk::docstore::{Document, QueryOptions, QueryResult};

/// In-memory documents with the production default's filter, sort, and
/// pagination semantics; clones share one store.
///
/// Wraps [`DocStoreDefault`], so a query behaves
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
    backend: DocStoreDefault,
}

impl MemoryDocs {
    /// The wrapped host backend, for scenarios that also run the runtime.
    #[must_use]
    pub const fn backend(&self) -> &DocStoreDefault {
        &self.backend
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
    async fn get(&self, store: &str, id: &str) -> Result<Option<Document>> {
        self.backend.get(store.to_owned(), id.to_owned()).await
    }

    async fn insert(&self, store: &str, doc: &Document) -> Result<()> {
        self.backend.insert(store.to_owned(), doc.clone()).await
    }

    async fn put(&self, store: &str, doc: &Document) -> Result<()> {
        self.backend.put(store.to_owned(), doc.clone()).await
    }

    async fn delete(&self, store: &str, id: &str) -> Result<bool> {
        self.backend.delete(store.to_owned(), id.to_owned()).await
    }

    async fn query(&self, store: &str, options: QueryOptions) -> Result<QueryResult> {
        self.backend.query(store.to_owned(), options).await
    }
}
