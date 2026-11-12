//! End-to-end tests for `omnia:docstore`: every scenario runs a real guest
//! component from `crates/test-programs` through the omnia runtime against
//! the in-memory default. The guest asserts what it observes across the
//! boundary (and traps on failure); the host side asserts what persisted in
//! the shared store after the run.

#![cfg(not(target_arch = "wasm32"))]

use std::sync::{Arc, Mutex};

use omnia::ExitStatus;
use omnia_test::host::{Backends, Deployment};
use omnia_wasi_docstore::{
    DocStoreClient, DocStoreDefault, Document, Error, FilterTree, FutureResult, QueryOpts,
    QueryResult, WasiDocStore,
};
use serde_json::{Value, json};

// Every guest program in `crates/test-programs` must have a matching test
// here; a new program without one fails to compile.
test_programs::foreach_docstore!();

async fn run_guest(wasm: &str, backends: Backends) {
    let status = Deployment::new()
        .guest("guest", wasm)
        .run_host::<WasiDocStore, _>(backends)
        .await
        .expect("guest runs");
    assert_eq!(status, ExitStatus::SUCCESS, "guest `{wasm}` failed");
}

async fn body(backends: &Backends, collection: &str, id: &str) -> Option<Value> {
    let doc = backends.docstore.get(collection.to_owned(), id.to_owned()).await.expect("get")?;
    assert_eq!(doc.id, id);
    Some(serde_json::from_slice(&doc.data).expect("JSON body"))
}

#[tokio::test]
async fn docstore_crud_query() {
    let backends = Backends::defaults().await;

    run_guest(test_programs::DOCSTORE_CRUD_QUERY, backends.clone()).await;

    // Persisted state: the inserts landed, the overwrite and the upsert took,
    // and the deleted document is gone.
    assert_eq!(
        body(&backends, "items", "a").await,
        Some(json!({"kind": "fruit", "rank": 3, "zone": "z1"}))
    );
    assert_eq!(body(&backends, "items", "b").await, None);
    assert_eq!(
        body(&backends, "items", "c").await,
        Some(json!({"kind": "veg", "rank": 9, "zone": "z2", "fresh": true}))
    );
    assert_eq!(body(&backends, "items", "f").await, Some(json!({"kind": "veg", "rank": 0})));
}

#[tokio::test]
async fn docstore_document_db() {
    run_guest(test_programs::DOCSTORE_DOCUMENT_DB, Backends::defaults().await).await;
}

// fails every call on a collection named for a kind with that kind,
// delegating any other collection, and records each collection a call
// reaches so the run's exchanges are pinned
#[derive(Clone, Debug, Default)]
struct Faulting {
    inner: DocStoreDefault,
    collections: Arc<Mutex<Vec<String>>>,
}

impl Faulting {
    fn fault(&self, collection: &str) -> Option<anyhow::Error> {
        self.collections.lock().expect("called collections").push(collection.to_owned());
        let fault = match collection {
            "missing" => Error::NotFound("missing in the suite".to_owned()).into(),
            "denied" => Error::Denied("denied by the suite".to_owned()).into(),
            "unsupported" => Error::Unsupported("unsupported by the suite".to_owned()).into(),
            "unavailable" => Error::Unavailable("unavailable in the suite".to_owned()).into(),
            "other" => anyhow::anyhow!("an untyped backend failure"),
            _ => return None,
        };
        Some(fault)
    }
}

impl DocStoreClient for Faulting {
    fn get(&self, collection: String, id: String) -> FutureResult<Option<Document>> {
        self.fault(&collection).map_or_else(|| self.inner.get(collection, id), failed)
    }

    fn insert(&self, collection: String, doc: Document) -> FutureResult<bool> {
        self.fault(&collection).map_or_else(|| self.inner.insert(collection, doc), failed)
    }

    fn put(&self, collection: String, doc: Document) -> FutureResult<()> {
        self.fault(&collection).map_or_else(|| self.inner.put(collection, doc), failed)
    }

    fn delete(&self, collection: String, id: String) -> FutureResult<bool> {
        self.fault(&collection).map_or_else(|| self.inner.delete(collection, id), failed)
    }

    fn query(
        &self, collection: String, filter: Option<FilterTree>, options: QueryOpts,
    ) -> FutureResult<QueryResult> {
        let fault = self.fault(&collection);
        fault.map_or_else(|| self.inner.query(collection, filter, options), failed)
    }
}

fn failed<T: 'static>(fault: anyhow::Error) -> FutureResult<T> {
    Box::pin(async move { Err(fault) })
}

#[tokio::test]
async fn docstore_error_kinds() {
    let backends = Backends::defaults().await.docstore(Faulting::default());

    let status = Deployment::new()
        .guest("guest", test_programs::DOCSTORE_ERROR_KINDS)
        .run_host::<WasiDocStore, _>(backends.clone())
        .await
        .expect("guest runs");
    assert_eq!(status, ExitStatus::SUCCESS, "guest failed");

    let collections = backends.docstore.collections.lock().expect("called collections").clone();
    assert_eq!(collections, ["bodies", "missing", "denied", "unsupported", "unavailable", "other"]);

    // the body the default refused was not stored
    let refused = backends.docstore.inner.get("bodies".to_owned(), "raw".to_owned()).await;
    assert!(refused.expect("get").is_none());
}
