//! End-to-end tests for `wasi:docstore`: every scenario runs a real guest
//! component from `crates/test-programs` through the omnia runtime against
//! the in-memory default. The guest asserts what it observes across the
//! boundary (and traps on failure); the host side asserts what persisted in
//! the shared store after the run.

#![cfg(not(target_arch = "wasm32"))]

use omnia::ExitStatus;
use omnia_test::host::{Backends, Deployment};
use omnia_wasi_docstore::{WasiDocStore, WasiDocStoreCtx as _};
use serde_json::{Value, json};

// Every guest program in `crates/test-programs` must have a matching test
// here; a new program without one fails to compile.
test_programs::foreach_docstore!();

/// Run one guest program against `backends`, requiring a clean exit.
async fn run_guest(wasm: &str, backends: Backends) {
    let status = Deployment::new()
        .guest("guest", wasm)
        .run_host::<WasiDocStore, _>(backends)
        .await
        .expect("guest runs");
    assert_eq!(status, ExitStatus::SUCCESS, "guest `{wasm}` failed");
}

/// One document's JSON body, read through the docstore handle.
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
async fn docstore_insert_conflict() {
    let backends = Backends::defaults().await;

    run_guest(test_programs::DOCSTORE_INSERT_CONFLICT, backends.clone()).await;

    // Persisted state: only the sanctioned overwrite made it past the first
    // insert.
    assert_eq!(body(&backends, "conflict", "dup").await, Some(json!({"v": 3})));
}
