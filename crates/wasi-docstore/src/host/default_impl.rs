//! Default in-memory implementation of [`DocStoreClient`](crate::host::DocStoreClient).
//!
//! Documents live in a clone-shared map of collections; state is process-local
//! and lost on exit, matching the other in-memory defaults. Filters are
//! evaluated directly over the stored JSON (see [`filter`]).

mod filter;

use std::collections::btree_map::Entry;
use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, RwLock};

use anyhow::{Context, Result};
use futures::FutureExt;
use omnia_core::Connect;
use serde_json::Value;
use tracing::instrument;

use crate::host::resource::FilterTree;
use crate::host::{DocStoreClient, Document, Error, FutureResult, QueryOpts, QueryResult};

const MAX_PAGE_SIZE: u64 = 1000;

/// Default in-memory [`DocStoreClient`].
///
/// Clones share the same store, so a probe handle kept by a test observes the
/// guest's writes.
#[derive(Clone, Default)]
pub struct DocStoreDefault {
    collections: Arc<RwLock<Collections>>,
}

impl DocStoreDefault {
    fn read(&self) -> std::sync::RwLockReadGuard<'_, Collections> {
        self.collections.read().expect("docstore lock poisoned")
    }

    fn write(&self) -> std::sync::RwLockWriteGuard<'_, Collections> {
        self.collections.write().expect("docstore lock poisoned")
    }
}

impl std::fmt::Debug for DocStoreDefault {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DocStoreDefault").finish_non_exhaustive()
    }
}

impl Connect for DocStoreDefault {
    type ConnectOptions = omnia_core::NoOptions;

    #[instrument]
    async fn connect_with(options: Self::ConnectOptions) -> Result<Self> {
        tracing::debug!("initializing in-memory document store");
        Ok(Self::default())
    }
}

impl DocStoreClient for DocStoreDefault {
    fn get(&self, collection: String, id: String) -> FutureResult<Option<Document>> {
        let store = self.clone();
        async move {
            let guard = store.read();
            guard
                .get(&collection)
                .and_then(|col| col.get(&id))
                .map(|body| to_document(&id, body))
                .transpose()
        }
        .boxed()
    }

    fn insert(&self, collection: String, doc: Document) -> FutureResult<bool> {
        let store = self.clone();
        async move {
            let body = parse_body(&doc)?;
            let inserted = match store.write().entry(collection).or_default().entry(doc.id) {
                Entry::Vacant(slot) => {
                    slot.insert(body);
                    true
                }
                Entry::Occupied(_) => false,
            };
            Ok(inserted)
        }
        .boxed()
    }

    fn put(&self, collection: String, doc: Document) -> FutureResult<()> {
        let store = self.clone();
        async move {
            let body = parse_body(&doc)?;
            store.write().entry(collection).or_default().insert(doc.id, body);
            Ok(())
        }
        .boxed()
    }

    fn delete(&self, collection: String, id: String) -> FutureResult<bool> {
        let store = self.clone();
        async move {
            Ok(store.write().get_mut(&collection).is_some_and(|col| col.remove(&id).is_some()))
        }
        .boxed()
    }

    fn query(
        &self, collection: String, filter: Option<FilterTree>, options: QueryOpts,
    ) -> FutureResult<QueryResult> {
        let store = self.clone();
        async move {
            // clamp the guest-supplied page size to the host maximum
            let limit = options.limit.map_or(MAX_PAGE_SIZE, u64::from).min(MAX_PAGE_SIZE);
            if limit == 0 {
                return Err(Error::Invalid("query limit must be at least 1".to_owned()).into());
            }
            let page_size = usize::try_from(limit).unwrap_or(usize::MAX);

            let skip = parse_continuation(options.continuation.as_deref())?
                + u64::from(options.offset.unwrap_or(0));
            let skip = usize::try_from(skip).unwrap_or(usize::MAX);

            let mut rows: Vec<(String, Value)> = {
                let guard = store.read();
                guard.get(&collection).map_or_else(Vec::new, |col| {
                    col.iter()
                        .filter(|(_, body)| {
                            filter.as_ref().is_none_or(|f| filter::matches(f, body))
                        })
                        .map(|(id, body)| (id.clone(), body.clone()))
                        .collect()
                })
            };

            if !options.order_by.is_empty() {
                // the id tiebreaker keeps pagination stable across equal sort keys
                rows.sort_by(|a, b| {
                    filter::compare_documents(&a.1, &b.1, &options.order_by)
                        .then_with(|| a.0.cmp(&b.0))
                });
            }

            let (page, continuation) = paginate(rows, skip, page_size);
            let documents =
                page.iter().map(|(id, body)| to_document(id, body)).collect::<Result<Vec<_>>>()?;

            Ok(QueryResult {
                documents,
                continuation,
            })
        }
        .boxed()
    }
}

// Collections keyed by name; documents keyed by id (BTreeMap gives queries a
// deterministic id-ascending base order).
type Collections = HashMap<String, BTreeMap<String, Value>>;

fn to_document(id: &str, body: &Value) -> Result<Document> {
    Ok(Document {
        id: id.to_owned(),
        data: serde_json::to_vec(body).context("serialize JSON body")?,
    })
}

fn parse_body(doc: &Document) -> Result<Value> {
    serde_json::from_slice(&doc.data)
        .map_err(|err| Error::Invalid(format!("document `{}` is not JSON: {err}", doc.id)).into())
}

fn parse_continuation(continuation: Option<&str>) -> Result<u64> {
    continuation.map_or_else(
        || Ok(0),
        |s| {
            s.parse().map_err(|err| {
                Error::Invalid(format!("invalid continuation token {s:?}: {err}")).into()
            })
        },
    )
}

// Take one page from the filtered/sorted rows, returning the continuation
// token (the absolute index of the next row) when more rows remain.
fn paginate(
    rows: Vec<(String, Value)>, skip: usize, page_size: usize,
) -> (Vec<(String, Value)>, Option<String>) {
    let total = rows.len();
    let page: Vec<_> = rows.into_iter().skip(skip).take(page_size).collect();
    let next = skip.saturating_add(page.len());
    let continuation = (next < total).then(|| next.to_string());
    (page, continuation)
}
