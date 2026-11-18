//! In-memory `DocumentDb` with the SDK's reference query semantics.

use std::cmp::Ordering;
use std::collections::btree_map::Entry;
use std::collections::{BTreeMap, HashMap};
use std::future::{Future, ready};
use std::sync::{Arc, RwLock, RwLockReadGuard, RwLockWriteGuard};

use omnia_sdk::docstore::{Document, Exists, Query};
use omnia_sdk::{DocumentDb, Error, Page, Result};
use serde_json::Value;

/// In-memory documents queried with the SDK's reference semantics; clones share one store.
///
/// A query behaves as it does through the runtime's in-memory backend:
///
/// - [`Filter::matches`](omnia_sdk::docstore::Filter::matches) selects the
///   documents.
/// - [`SortField::compare`](omnia_sdk::docstore::SortField::compare) orders
///   them field by field, with ties broken on the id.
/// - A `limit` is capped at 1000, and a `limit` of zero fails with the code
///   `invalid`.
/// - A filter [`Filter::check`](omnia_sdk::docstore::Filter::check) refuses
///   fails with the code `invalid`.
/// - An `offset` fails with the code `unsupported`.
/// - The `after` cursor resumes the query, and a malformed cursor fails with
///   the code `invalid`.
///
/// `insert` answers `Ok(Err(Exists))` for an id already stored. A write whose
/// body is not a JSON object fails with the code `invalid`.
///
/// ```
/// use omnia_sdk::DocumentDb as _;
/// use omnia_sdk::docstore::{Document, Filter, Query};
/// use omnia_test::guest::MemoryDocs;
///
/// # tokio::runtime::Runtime::new().unwrap().block_on(async {
/// let docs = MemoryDocs::default();
/// for (id, age) in [("a", 17), ("b", 42)] {
///     let document = Document::new(id, format!(r#"{{"age":{age}}}"#).as_bytes());
///     docs.put("people", &document).await.unwrap();
/// }
/// let query = Query {
///     filter: Some(Filter::gte("age", 18)),
///     ..Query::default()
/// };
/// let adults = docs.query("people", &query).await.unwrap();
/// assert_eq!(adults.items[0].id, "b");
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
    /// Every document in `collection`, in id order.
    ///
    /// # Errors
    ///
    /// A [`ServerError`](Error::ServerError) with the code `other` when a
    /// stored body does not serialize.
    pub fn documents(
        &self, collection: &str,
    ) -> impl Future<Output = Result<Vec<Document>>> + Send {
        let documents = self
            .read()
            .get(collection)
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

    fn insert_document(&self, collection: &str, doc: &Document) -> Result<Result<(), Exists>> {
        let body = body(doc)?;
        let mut collections = self.write();
        match collections.entry(collection.to_owned()).or_default().entry(doc.id.clone()) {
            Entry::Vacant(slot) => {
                slot.insert(body);
                Ok(Ok(()))
            }
            Entry::Occupied(_) => Ok(Err(Exists)),
        }
    }

    fn put_document(&self, collection: &str, doc: &Document) -> Result<()> {
        let body = body(doc)?;
        self.write().entry(collection.to_owned()).or_default().insert(doc.id.clone(), body);
        Ok(())
    }

    fn page(&self, collection: &str, query: &Query) -> Result<Page<Document>> {
        if query.offset.is_some() {
            return Err(Error::server_error("unsupported", "a document query takes no `offset`"));
        }
        let limit = query.limit.map_or(MAX_PAGE_SIZE, |limit| limit.min(MAX_PAGE_SIZE));
        if limit == 0 {
            return Err(Error::server_error("invalid", "query limit must be at least 1"));
        }
        if let Some(filter) = &query.filter {
            filter.check()?;
        }
        let page_size = usize::try_from(limit).unwrap_or(usize::MAX);
        let skip = cursor(query.after.as_deref())?;

        let mut rows: Vec<(String, Value)> = self
            .read()
            .get(collection)
            .into_iter()
            .flatten()
            .filter(|(_, body)| query.filter.as_ref().is_none_or(|filter| filter.matches(body)))
            .map(|(id, body)| (id.clone(), body.clone()))
            .collect();
        rows.sort_by(|(a_id, a), (b_id, b)| {
            query
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
            .collect::<Result<Vec<_>, _>>()?;
        let next = skip.saturating_add(documents.len());
        Ok(Page::new(documents, (next < total).then(|| next.to_string())))
    }
}

impl DocumentDb for MemoryDocs {
    fn get(
        &self, collection: &str, id: &str,
    ) -> impl Future<Output = Result<Option<Document>>> + Send {
        let found = self
            .read()
            .get(collection)
            .and_then(|documents| documents.get(id))
            .map(|body| document(id, body))
            .transpose();
        ready(found)
    }

    fn insert(
        &self, collection: &str, document: &Document,
    ) -> impl Future<Output = Result<Result<(), Exists>>> + Send {
        ready(self.insert_document(collection, document))
    }

    fn put(
        &self, collection: &str, document: &Document,
    ) -> impl Future<Output = Result<()>> + Send {
        ready(self.put_document(collection, document))
    }

    fn delete(&self, collection: &str, id: &str) -> impl Future<Output = Result<bool>> + Send {
        let removed = self
            .write()
            .get_mut(collection)
            .is_some_and(|documents| documents.remove(id).is_some());
        ready(Ok(removed))
    }

    fn query(
        &self, collection: &str, query: &Query,
    ) -> impl Future<Output = Result<Page<Document>>> + Send {
        ready(self.page(collection, query))
    }
}

fn cursor(after: Option<&str>) -> Result<usize> {
    after.map_or(Ok(0), |after| {
        after.parse().map_err(|err| {
            Error::server_error("invalid", format!("invalid cursor {after:?}: {err}"))
        })
    })
}

fn document(id: &str, body: &Value) -> Result<Document> {
    let data = serde_json::to_vec(body).map_err(|err| {
        Error::server_error("other", format!("serializing document `{id}`: {err}"))
    })?;
    Ok(Document::new(id, &data))
}

fn body(doc: &Document) -> Result<Value> {
    let id = &doc.id;
    match serde_json::from_slice(&doc.data) {
        Ok(body @ Value::Object(_)) => Ok(body),
        Ok(_) => Err(Error::server_error("invalid", format!("document `{id}` is not an object"))),
        Err(err) => {
            Err(Error::server_error("invalid", format!("document `{id}` is not JSON: {err}")))
        }
    }
}
