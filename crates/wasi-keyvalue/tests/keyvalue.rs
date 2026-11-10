//! End-to-end tests for `wasi:keyvalue`: every scenario runs a real guest
//! component from `crates/test-programs` through the omnia runtime against
//! the in-memory default, held in the slot bundle a list-form `runtime!`
//! builds. The guest asserts what it observes across the boundary (and traps
//! on failure); the host side seeds the bucket and asserts what persisted
//! after the run.

#![cfg(not(target_arch = "wasm32"))]

use std::sync::Arc;

use omnia::{ExitStatus, Slots};
use omnia_test::host::{Backends, Deployment};
use omnia_wasi_keyvalue::{Bucket, WasiKeyValue};

// Every guest program in `crates/test-programs` must have a matching test
// here; a new program without one fails to compile.
test_programs::foreach_keyvalue!();

// the keyvalue default, and the telemetry default every command guest imports
async fn slots() -> Slots {
    let defaults = Backends::defaults().await;
    let mut slots = Slots::new();
    slots.add(&defaults.keyvalue).expect("keyvalue slot");
    slots.add(&defaults.otel).expect("otel slot");
    slots
}

async fn bucket(slots: &Slots) -> Arc<dyn Bucket> {
    let keyvalue = slots.get::<WasiKeyValue>().expect("keyvalue slot");
    keyvalue.open_bucket("bucket".to_owned()).await.expect("bucket")
}

async fn run_guest(wasm: &str, slots: Slots) {
    let status = Deployment::new()
        .guest("guest", wasm)
        .run_host::<WasiKeyValue, _>(slots)
        .await
        .expect("guest runs");
    assert_eq!(status, ExitStatus::SUCCESS, "guest `{wasm}` failed");
}

#[tokio::test]
async fn keyvalue_bucket() {
    let slots = slots().await;
    let seeded = bucket(&slots).await;
    seeded.set("counter".to_owned(), 37_i64.to_be_bytes().to_vec(), None).await.expect("seed");

    run_guest(test_programs::KEYVALUE_BUCKET, slots.clone()).await;

    // Persisted state: the increments landed, the deletes took, and the CAS
    // retry's value is what survived.
    let bucket = bucket(&slots).await;
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

#[tokio::test]
async fn keyvalue_ttl_expires() {
    let slots = slots().await;

    run_guest(test_programs::KEYVALUE_TTL_EXPIRES, slots.clone()).await;

    // Raw bytes landed without an envelope; the expired key is gone.
    let bucket = bucket(&slots).await;
    assert_eq!(bucket.get("ephemeral".to_owned()).await.expect("get"), None);
    assert_eq!(bucket.get("live".to_owned()).await.expect("get"), Some(b"raw-bytes".to_vec()));
    assert_eq!(bucket.keys().await.expect("keys"), ["live"]);
}
