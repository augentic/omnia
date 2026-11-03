//! In-memory `DocumentStore` double for handler tests.

use std::collections::btree_map::Entry;
use std::collections::{BTreeMap, HashMap};
use std::fmt::Debug;
use std::future::{Future, ready};
use std::sync::{Arc, RwLock};

use anyhow::{Context, Result};
use omnia_backend::docstore::filter;
use omnia_sdk::DocumentStore;
use omnia_sdk::docstore::{Document, QueryOptions, QueryResult};
use serde_json::Value;

const MAX_PAGE_SIZE: u64 = 1000;

type Collections = HashMap<String, BTreeMap<String, Value>>;

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
    collections: Arc<RwLock<Collections>>,
}

impl Debug for MemoryDocs {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MemoryDocs").finish_non_exhaustive()
    }
}

impl MemoryDocs {
    fn read(&self) -> std::sync::RwLockReadGuard<'_, Collections> {
        self.collections.read().expect("docstore lock poisoned")
    }

    fn write(&self) -> std::sync::RwLockWriteGuard<'_, Collections> {
        self.collections.write().expect("docstore lock poisoned")
    }

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

    fn query_sync(&self, collection: &str, options: &QueryOptions) -> Result<QueryResult> {
        let limit = options.limit.map_or(MAX_PAGE_SIZE, u64::from).min(MAX_PAGE_SIZE);
        anyhow::ensure!(limit > 0, "query limit must be at least 1");
        let page_size = usize::try_from(limit).unwrap_or(usize::MAX);

        let skip = options
            .continuation
            .as_deref()
            .map_or_else(
                || Ok(0_u64),
                |s| s.parse().with_context(|| format!("invalid continuation token: {s:?}")),
            )?
            .saturating_add(u64::from(options.offset.unwrap_or(0)));
        let skip = usize::try_from(skip).unwrap_or(usize::MAX);

        let mut rows: Vec<(String, Value)> = {
            let guard = self.read();
            guard.get(collection).map_or_else(Vec::new, |col| {
                col.iter()
                    .filter(|(_, body)| {
                        options.filter.as_ref().is_none_or(|f| filter::matches(f, body))
                    })
                    .map(|(id, body)| (id.clone(), body.clone()))
                    .collect()
            })
        };

        if !options.order_by.is_empty() {
            rows.sort_by(|a, b| {
                filter::compare_documents(&a.1, &b.1, &options.order_by).then_with(|| a.0.cmp(&b.0))
            });
        }

        let total = rows.len();
        let page: Vec<_> = rows.into_iter().skip(skip).take(page_size).collect();
        let next = skip.saturating_add(page.len());
        let continuation = (next < total).then(|| next.to_string());
        let documents =
            page.iter().map(|(id, body)| to_document(id, body)).collect::<Result<Vec<_>>>()?;

        Ok(QueryResult {
            documents,
            continuation,
        })
    }
}

impl DocumentStore for MemoryDocs {
    fn get(&self, store: &str, id: &str) -> impl Future<Output = Result<Option<Document>>> + Send {
        let res = {
            let guard = self.read();
            guard
                .get(store)
                .and_then(|col| col.get(id))
                .map(|body| to_document(id, body))
                .transpose()
        };
        ready(res)
    }

    fn insert(&self, store: &str, doc: &Document) -> impl Future<Output = Result<()>> + Send {
        let res = (|| {
            let body = parse_body(doc)?;
            let inserted =
                match self.write().entry(store.to_owned()).or_default().entry(doc.id.clone()) {
                    Entry::Vacant(slot) => {
                        slot.insert(body);
                        true
                    }
                    Entry::Occupied(_) => false,
                };
            anyhow::ensure!(inserted, "document id already exists");
            Ok(())
        })();
        ready(res)
    }

    fn put(&self, store: &str, doc: &Document) -> impl Future<Output = Result<()>> + Send {
        let res = (|| {
            let body = parse_body(doc)?;
            self.write().entry(store.to_owned()).or_default().insert(doc.id.clone(), body);
            Ok(())
        })();
        ready(res)
    }

    fn delete(&self, store: &str, id: &str) -> impl Future<Output = Result<bool>> + Send {
        let removed = self.write().get_mut(store).is_some_and(|col| col.remove(id).is_some());
        ready(Ok(removed))
    }

    fn query(
        &self, store: &str, options: QueryOptions,
    ) -> impl Future<Output = Result<QueryResult>> + Send {
        ready(self.query_sync(store, &options))
    }
}

fn parse_body(doc: &Document) -> Result<Value> {
    serde_json::from_slice(&doc.data).context("invalid JSON in document body")
}

fn to_document(id: &str, body: &Value) -> Result<Document> {
    Ok(Document {
        id: id.to_owned(),
        data: serde_json::to_vec(body).context("serialize JSON body")?,
    })
}
