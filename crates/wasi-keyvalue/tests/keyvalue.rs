//! End-to-end tests for `omnia:keyvalue`: every scenario runs a real guest
//! component from `crates/test-programs` through the omnia runtime against
//! the in-memory default. The guest asserts what it observes across the
//! boundary (and traps on failure); the host side seeds the bucket and
//! asserts what persisted after the run.

#![cfg(not(target_arch = "wasm32"))]

use std::sync::{Arc, Mutex};

use omnia::{Connect as _, ExitStatus};
use omnia_test::host::{Backends, Deployment};
use omnia_wasi_keyvalue::{
    Bucket, ConnectOptions, Error, FutureResult, KeyValueClient, KeyValueDefault, Lifetime,
    WasiKeyValue,
};

// Every guest program in `crates/test-programs` must have a matching test
// here; a new program without one fails to compile.
test_programs::foreach_keyvalue!();

async fn run_guest(wasm: &str, backends: Backends) {
    let status = Deployment::new()
        .guest("guest", wasm)
        .run_host::<WasiKeyValue, _>(backends)
        .await
        .expect("guest runs");
    assert_eq!(status, ExitStatus::SUCCESS, "guest `{wasm}` failed");
}

async fn declaring(buckets: &[&str]) -> KeyValueDefault {
    let options = ConnectOptions {
        buckets: buckets.iter().map(|&bucket| bucket.to_owned()).collect(),
    };
    KeyValueDefault::connect_with(options).await.expect("in-memory keyvalue")
}

#[tokio::test]
async fn keyvalue_bucket() {
    let backends = Backends::defaults().await.keyvalue(declaring(&["bucket"]).await);
    let bucket = backends.keyvalue.open_bucket("bucket".to_owned()).await.expect("bucket");
    let seeded = bucket.set("counter".to_owned(), b"37".to_vec(), Lifetime::Replace(None), None);
    seeded.await.expect("seed").expect("no precondition");

    run_guest(test_programs::KEYVALUE_BUCKET, backends.clone()).await;

    // Persisted state: the deletes took, the refused calls wrote nothing,
    // and the counters are decimal text.
    let bucket = backends.keyvalue.open_bucket("bucket".to_owned()).await.expect("bucket");
    let mut keys = bucket.keys().await.expect("keys");
    keys.sort();
    assert_eq!(keys, ["counter", "fresh", "kept", "lock", "padded", "page.1", "page.2", "page.3"]);
    let counter = bucket.get("counter".to_owned()).await.expect("get").map(|entry| entry.value);
    assert_eq!(counter, Some(b"42".to_vec()));
    let fresh = bucket.get("fresh".to_owned()).await.expect("get").map(|entry| entry.value);
    assert_eq!(fresh, Some(b"3".to_vec()));
}

#[tokio::test]
async fn keyvalue_state_store() {
    // `omnia_test::contract::state` runs over the store's own bucket and `state::SECOND`
    let backends = Backends::defaults().await.keyvalue(declaring(&["default", "second"]).await);

    run_guest(test_programs::KEYVALUE_STATE_STORE, backends.clone()).await;

    // the guest's bytes are stored as they are, and the host expires them
    assert_eq!(backends.state("host.raw").await, Some(b"raw".to_vec()));
    assert_eq!(backends.state("host.persist").await, Some(b"kept".to_vec()));
    assert_eq!(backends.state("host.expiring").await, None);
}

// fails `open_bucket` with the kind a bucket is named for, delegating any
// other name, and records each name so the run's exchanges are pinned
#[derive(Clone, Debug)]
struct Faulting {
    inner: KeyValueDefault,
    opened: Arc<Mutex<Vec<String>>>,
}

impl KeyValueClient for Faulting {
    fn open_bucket(&self, identifier: String) -> FutureResult<Arc<dyn Bucket>> {
        self.opened.lock().expect("opened buckets").push(identifier.clone());
        let fault = match identifier.as_str() {
            "denied" => Error::Denied("denied by the suite".to_owned()).into(),
            "unsupported" => Error::Unsupported("unsupported by the suite".to_owned()).into(),
            "unavailable" => Error::Unavailable("unavailable in the suite".to_owned()).into(),
            "other" => anyhow::anyhow!("an untyped backend failure"),
            _ => return self.inner.open_bucket(identifier),
        };
        Box::pin(async move { Err(fault) })
    }
}

#[tokio::test]
async fn keyvalue_error_codes() {
    let faulting = Faulting {
        inner: declaring(&["default"]).await,
        opened: Arc::default(),
    };
    let backends = Backends::defaults().await.keyvalue(faulting);

    let status = Deployment::new()
        .guest("guest", test_programs::KEYVALUE_ERROR_CODES)
        .run_host::<WasiKeyValue, _>(backends.clone())
        .await
        .expect("guest runs");
    assert_eq!(status, ExitStatus::SUCCESS, "guest failed");

    // the SDK opens a call's bucket before the host checks its TTL
    let opened = backends.keyvalue.opened.lock().expect("opened buckets").clone();
    assert_eq!(opened, ["default", "undeclared", "denied", "unsupported", "unavailable", "other"]);
}
