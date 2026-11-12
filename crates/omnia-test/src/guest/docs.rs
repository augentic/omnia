//! In-memory `DocumentStore` with the SDK's reference query semantics.

use std::cmp::Ordering;
use std::collections::btree_map::Entry;
use std::collections::{BTreeMap, HashMap};
use std::future::{Future, ready};
use std::sync::{Arc, RwLock, RwLockReadGuard, RwLockWriteGuard};

use anyhow::{Context as _, Result, ensure};
use omnia_sdk::DocumentStore;
use omnia_sdk::docstore::{Document, QueryOptions, QueryResult};
use serde_json::Value;

/// In-memory documents queried with the SDK's reference semantics; clones share one store.
///
/// A query behaves as it does through the runtime's in-memory backend:
///
/// - [`Filter::matches`](omnia_sdk::docstore::Filter::matches) selects the
///   documents.
/// - [`SortField::compare`](omnia_sdk::docstore::SortField::compare) orders
///   them field by field, with ties broken on the id.
/// - A `limit` is capped at 1000, and a `limit` of zero is an error.
/// - The `offset` and the continuation token skip documents, and a malformed
///   token is an error.
///
/// `insert` refuses an id already stored, and every body must be JSON.
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
    collections: Arc<RwLock<Collections>>,
}

// documents keyed by id, so a query's base order is id-ascending
type Collections = HashMap<String, BTreeMap<String, Value>>;

const MAX_PAGE_SIZE: u32 = 1000;

impl MemoryDocs {
    /// Every document in `store`, in id order.
    ///
    /// # Errors
    ///
    /// Returns an error when a stored body does not serialize.
    pub fn documents(&self, store: &str) -> impl Future<Output = Result<Vec<Document>>> + Send {
        let documents = self
            .read()
            .get(store)
            .into_iter()
            .flatten()
            .map(|(id, body)| document(id, body))
            .collect();
        ready(documents)
    }

    fn read(&self) -> RwLockReadGuard<'_, Collections> {
        self.collections.read().expect("docs lock")
    }

    fn write(&self) -> RwLockWriteGuard<'_, Collections> {
        self.collections.write().expect("docs lock")
    }

    fn insert_document(&self, store: &str, doc: &Document) -> Result<()> {
        let body = body(doc)?;
        let inserted = match self.write().entry(store.to_owned()).or_default().entry(doc.id.clone())
        {
            Entry::Vacant(slot) => {
                slot.insert(body);
                true
            }
            Entry::Occupied(_) => false,
        };
        ensure!(inserted, "document id already exists");
        Ok(())
    }

    fn put_document(&self, store: &str, doc: &Document) -> Result<()> {
        let body = body(doc)?;
        self.write().entry(store.to_owned()).or_default().insert(doc.id.clone(), body);
        Ok(())
    }

    fn page(&self, store: &str, options: &QueryOptions) -> Result<QueryResult> {
        let limit = options.limit.map_or(MAX_PAGE_SIZE, |limit| limit.min(MAX_PAGE_SIZE));
        ensure!(limit > 0, "query limit must be at least 1");
        let page_size = usize::try_from(limit).unwrap_or(usize::MAX);
        let offset =
            options.offset.map_or(0, |offset| usize::try_from(offset).unwrap_or(usize::MAX));
        let skip = continuation(options.continuation.as_deref())?.saturating_add(offset);

        let mut rows: Vec<(String, Value)> = self
            .read()
            .get(store)
            .into_iter()
            .flatten()
            .filter(|(_, body)| options.filter.as_ref().is_none_or(|filter| filter.matches(body)))
            .map(|(id, body)| (id.clone(), body.clone()))
            .collect();
        rows.sort_by(|(a_id, a), (b_id, b)| {
            options
                .order_by
                .iter()
                .fold(Ordering::Equal, |ordering, sort| ordering.then_with(|| sort.compare(a, b)))
                .then_with(|| a_id.cmp(b_id))
        });

        let total = rows.len();
        let documents = rows
            .iter()
            .skip(skip)
            .take(page_size)
            .map(|(id, body)| document(id, body))
            .collect::<Result<Vec<_>>>()?;
        let next = skip.saturating_add(documents.len());
        Ok(QueryResult {
            documents,
            continuation: (next < total).then(|| next.to_string()),
        })
    }
}

impl DocumentStore for MemoryDocs {
    fn get(&self, store: &str, id: &str) -> impl Future<Output = Result<Option<Document>>> + Send {
        let found = self
            .read()
            .get(store)
            .and_then(|documents| documents.get(id))
            .map(|body| document(id, body))
            .transpose();
        ready(found)
    }

    fn insert(&self, store: &str, doc: &Document) -> impl Future<Output = Result<()>> + Send {
        ready(self.insert_document(store, doc))
    }

    fn put(&self, store: &str, doc: &Document) -> impl Future<Output = Result<()>> + Send {
        ready(self.put_document(store, doc))
    }

    fn delete(&self, store: &str, id: &str) -> impl Future<Output = Result<bool>> + Send {
        let removed =
            self.write().get_mut(store).is_some_and(|documents| documents.remove(id).is_some());
        ready(Ok(removed))
    }

    fn query(
        &self, store: &str, options: QueryOptions,
    ) -> impl Future<Output = Result<QueryResult>> + Send {
        ready(self.page(store, &options))
    }
}

fn continuation(token: Option<&str>) -> Result<usize> {
    token.map_or(Ok(0), |token| {
        token.parse().with_context(|| format!("invalid continuation token: {token:?}"))
    })
}

fn document(id: &str, body: &Value) -> Result<Document> {
    Ok(Document {
        id: id.to_owned(),
        data: serde_json::to_vec(body).context("serialize JSON body")?,
    })
}

fn body(doc: &Document) -> Result<Value> {
    serde_json::from_slice(&doc.data).context("invalid JSON in document body")
}
