//! End-to-end tests for `wasi:keyvalue`: every scenario runs a real guest
//! component from `crates/test-programs` through the omnia runtime against
//! the in-memory default. The guest asserts what it observes across the
//! boundary (and traps on failure); the host side seeds the bucket and
//! asserts what persisted after the run.

#![cfg(not(target_arch = "wasm32"))]

use omnia::ExitStatus;
use omnia_test::host::{Backends, Deployment};
use omnia_wasi_keyvalue::{WasiKeyValue, WasiKeyValueCtx as _};

// Every guest program in `crates/test-programs` must have a matching test
// here; a new program without one fails to compile.
test_programs::foreach_keyvalue!();

/// Run one guest program against `backends`, requiring a clean exit.
async fn run_guest(wasm: &str, backends: Backends) {
    let status = Deployment::new()
        .guest("guest", wasm)
        .run_host::<WasiKeyValue, _>(backends)
        .await
        .expect("guest runs");
    assert_eq!(status, ExitStatus::SUCCESS, "guest `{wasm}` failed");
}

#[tokio::test]
async fn keyvalue_bucket() {
    let backends = Backends::defaults().await;
    let bucket = backends.keyvalue.open_bucket("bucket".to_owned()).await.expect("bucket");
    bucket.set("counter".to_owned(), 37_i64.to_be_bytes().to_vec()).await.expect("seed");

    run_guest(test_programs::KEYVALUE_BUCKET, backends.clone()).await;

    // Persisted state: the increments landed, the deletes took, and the CAS
    // retry's value is what survived.
    let bucket = backends.keyvalue.open_bucket("bucket".to_owned()).await.expect("bucket");
    let mut keys = bucket.keys().await.expect("keys");
    keys.sort();
    assert_eq!(keys, ["b", "cas", "counter", "fresh"]);
    assert_eq!(
        bucket.get("counter".to_owned()).await.expect("get"),
        Some(42_i64.to_be_bytes().to_vec())
    );
    assert_eq!(
        bucket.get("fresh".to_owned()).await.expect("get"),
        Some(3_i64.to_be_bytes().to_vec())
    );
    assert_eq!(bucket.get("b".to_owned()).await.expect("get"), Some(b"2".to_vec()));
    assert_eq!(bucket.get("cas".to_owned()).await.expect("get"), Some(b"retried".to_vec()));
}
