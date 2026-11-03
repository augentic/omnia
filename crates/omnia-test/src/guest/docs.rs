//! In-memory `DocumentStore` double for handler tests.

use std::fmt::Debug;
use std::future::{Future, ready};

use anyhow::Result;
use omnia_backend::docstore::filter::MemoryStore;
use omnia_sdk::DocumentStore;
use omnia_sdk::docstore::{Document, QueryOptions, QueryResult};

/// In-memory documents with the runtime default's filter, sort, and
/// pagination semantics; clones share one store.
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
#[derive(Clone, Default)]
pub struct MemoryDocs {
    store: MemoryStore,
}

impl Debug for MemoryDocs {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MemoryDocs").finish_non_exhaustive()
    }
}

impl MemoryDocs {
    /// Every document in `store`, following continuations to the end.
    ///
    /// # Errors
    ///
    /// Returns the store's error.
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
    fn get(&self, store: &str, id: &str) -> impl Future<Output = Result<Option<Document>>> + Send {
        ready(self.store.get(store, id))
    }

    fn insert(&self, store: &str, doc: &Document) -> impl Future<Output = Result<()>> + Send {
        ready(self.store.insert(store, doc))
    }

    fn put(&self, store: &str, doc: &Document) -> impl Future<Output = Result<()>> + Send {
        ready(self.store.put(store, doc))
    }

    fn delete(&self, store: &str, id: &str) -> impl Future<Output = Result<bool>> + Send {
        ready(Ok(self.store.delete(store, id)))
    }

    fn query(
        &self, store: &str, options: QueryOptions,
    ) -> impl Future<Output = Result<QueryResult>> + Send {
        ready(self.store.query(store, &options))
    }
}
