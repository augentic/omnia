//! Default in-memory implementation of [`DocStoreClient`](crate::DocStoreClient).
//!
//! Documents live in a clone-shared map of collections; state is process-local
//! and lost on exit, matching the other in-memory defaults. Filters are
//! evaluated directly over the stored JSON (see [`filter`]).

mod filter;

use std::cmp::Ordering;
use std::collections::btree_map::Entry;
use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, RwLock, RwLockReadGuard, RwLockWriteGuard};

use anyhow::{Context as _, Result};
use futures::FutureExt;
use omnia_core::{Connect, FromEnv};
use serde_json::Value;
use tracing::instrument;

use crate::{
    DocStoreClient, Document, Error, Filter, FutureResult, QueryOpts, QueryResult, SortField,
};

/// Options used to connect to the in-memory document store.
///
/// This struct is used to load connection options from environment variables.
#[derive(Debug, Clone, FromEnv)]
pub struct ConnectOptions {
    /// The collections a guest may open (`DOCSTORE_COLLECTIONS`, comma-separated, default: `default`).
    #[env(from = "DOCSTORE_COLLECTIONS", default = "default", with = omnia_core::env::list)]
    pub collections: Vec<String>,
}

/// Default in-memory [`DocStoreClient`].
///
/// It serves the collections its [`ConnectOptions`] declares, and refuses
/// every other name with [`Error::NotFound`]. It fails a query that holds a
/// [`Test::ArrayContains`](crate::Test::ArrayContains) with
/// [`Error::Unsupported`]. Clones share the same store, so a probe handle
/// kept by a test observes the guest's writes.
#[derive(Clone)]
pub struct DocStoreDefault {
    collections: Arc<RwLock<Collections>>,
}

impl DocStoreDefault {
    fn read(&self) -> RwLockReadGuard<'_, Collections> {
        self.collections.read().expect("docstore lock poisoned")
    }

    fn write(&self) -> RwLockWriteGuard<'_, Collections> {
        self.collections.write().expect("docstore lock poisoned")
    }
}

impl std::fmt::Debug for DocStoreDefault {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DocStoreDefault").finish_non_exhaustive()
    }
}

impl Connect for DocStoreDefault {
    type ConnectOptions = ConnectOptions;

    #[instrument]
    async fn connect_with(options: Self::ConnectOptions) -> Result<Self> {
        tracing::debug!("initializing in-memory document store");
        let collections =
            options.collections.into_iter().map(|name| (name, BTreeMap::new())).collect();
        Ok(Self {
            collections: Arc::new(RwLock::new(Collections(collections))),
        })
    }
}

impl DocStoreClient for DocStoreDefault {
    fn get(&self, collection: String, id: String) -> FutureResult<Option<Document>> {
        let store = self.clone();
        async move {
            let guard = store.read();
            guard.documents(&collection)?.get(&id).map(|body| to_document(&id, body)).transpose()
        }
        .boxed()
    }

    fn insert(&self, collection: String, doc: Document) -> FutureResult<bool> {
        let store = self.clone();
        async move {
            let body = parse_body(&doc)?;
            let inserted = match store.write().documents_mut(&collection)?.entry(doc.id) {
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
            store.write().documents_mut(&collection)?.insert(doc.id, body);
            Ok(())
        }
        .boxed()
    }

    fn delete(&self, collection: String, id: String) -> FutureResult<bool> {
        let store = self.clone();
        async move { Ok(store.write().documents_mut(&collection)?.remove(&id).is_some()) }.boxed()
    }

    fn query(
        &self, collection: String, filter: Filter, options: QueryOpts,
    ) -> FutureResult<QueryResult> {
        let store = self.clone();
        async move {
            if let Some(field) = filter::array_test(&filter) {
                let message =
                    format!("the in-memory docstore cannot test the elements of `{field}`");
                return Err(Error::Unsupported(message).into());
            }
            let page_size = usize::try_from(options.limit).unwrap_or(usize::MAX);
            let order_by = &options.order_by;
            let after = options.continuation.as_deref().map(|cursor| Row::decode(cursor, order_by));
            let after = after.transpose()?;

            // the matches in a total order: their `order-by` values, then their ids
            let mut rows: Vec<Row> = {
                let guard = store.read();
                guard
                    .documents(&collection)?
                    .iter()
                    .filter(|(_, body)| filter::matches(&filter, body))
                    .map(|(id, body)| Row::new(id, body, order_by))
                    .collect()
            };
            rows.sort_by(|a, b| a.cmp(b, order_by));

            // resume after the last document the previous page returned, so a
            // write between pages shifts no unchanged document
            let start = after.map_or(0, |after| {
                rows.partition_point(|row| row.cmp(&after, order_by) != Ordering::Greater)
            });
            let rest = &rows[start..];
            let page = &rest[..rest.len().min(page_size)];

            let continuation = page.last().filter(|_| page.len() < rest.len()).map(Row::encode);
            let documents = page
                .iter()
                .map(|row| to_document(&row.id, &row.body))
                .collect::<Result<Vec<_>>>()?;
            Ok(QueryResult {
                documents,
                continuation,
            })
        }
        .boxed()
    }
}

// Documents keyed by id in each declared collection. A `BTreeMap` gives a
// query a deterministic id-ascending base order.
struct Collections(HashMap<String, BTreeMap<String, Value>>);

impl Collections {
    fn documents(&self, name: &str) -> Result<&BTreeMap<String, Value>> {
        self.0.get(name).ok_or_else(|| undeclared(name))
    }

    fn documents_mut(&mut self, name: &str) -> Result<&mut BTreeMap<String, Value>> {
        self.0.get_mut(name).ok_or_else(|| undeclared(name))
    }
}

fn undeclared(name: &str) -> anyhow::Error {
    Error::NotFound(format!("collection `{name}` is not declared")).into()
}

// A matching document and the values it orders by. A page's cursor is its
// last row's `key` and `id`, so the next page starts after that position.
struct Row {
    id: String,
    key: Vec<Value>,
    body: Value,
}

impl Row {
    fn new(id: &str, body: &Value, order_by: &[SortField]) -> Self {
        Self {
            id: id.to_owned(),
            key: filter::sort_key(body, order_by),
            body: body.clone(),
        }
    }

    fn cmp(&self, other: &Self, order_by: &[SortField]) -> Ordering {
        filter::compare_keys(&self.key, &other.key, order_by).then_with(|| self.id.cmp(&other.id))
    }

    fn encode(&self) -> String {
        Value::Array(vec![Value::Array(self.key.clone()), Value::String(self.id.clone())])
            .to_string()
    }

    fn decode(cursor: &str, order_by: &[SortField]) -> Result<Self> {
        let (key, id): (Vec<Value>, String) = serde_json::from_str(cursor)
            .map_err(|err| Error::Invalid(format!("invalid cursor {cursor:?}: {err}")))?;
        if key.len() != order_by.len() {
            let message = format!("cursor {cursor:?} is from a query with another order");
            return Err(Error::Invalid(message).into());
        }
        Ok(Self {
            id,
            key,
            body: Value::Null,
        })
    }
}

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
