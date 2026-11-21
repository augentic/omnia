//! End-to-end tests for `omnia:keyvalue`: every scenario runs a real guest
//! component from `crates/test-programs` through the omnia runtime against
//! the in-memory default. The guest asserts what it observes across the
//! boundary (and traps on failure); the host side seeds the bucket and
//! asserts what persisted after the run.

#![cfg(not(target_arch = "wasm32"))]

use std::sync::{Arc, Mutex};
use std::time::Duration;

use omnia::{Connect as _, ExitStatus};
use omnia_test::host::{Backends, Deployment};
use omnia_wasi_keyvalue::{
    Bucket, ConnectOptions, Entry, Error, FutureResult, KeyValueClient, KeyValueDefault, Lifetime,
    Precondition, Revision, WasiKeyValue,
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

#[tokio::test]
async fn keyvalue_update_raced() {
    let racing = Racing {
        inner: declaring(&["default"]).await,
        calls: Arc::default(),
        raced: Arc::default(),
    };
    let backends = Backends::defaults().await.keyvalue(racing);
    let bucket = backends.keyvalue.inner.open_bucket("default".to_owned()).await.expect("bucket");
    let seeded = bucket.set("visits".to_owned(), b"1".to_vec(), Lifetime::Replace(None), None);
    let seeded = seeded.await.expect("seed").expect("no precondition");

    let status = Deployment::new()
        .guest("guest", test_programs::KEYVALUE_UPDATE_RACED)
        .run_host::<WasiKeyValue, _>(backends.clone())
        .await
        .expect("guest runs");
    assert_eq!(status, ExitStatus::SUCCESS, "guest failed");

    // one read, then each store under the revision of what it read: the
    // seed, then the racing write the conflict carried back
    let raced = backends.keyvalue.raced.lock().expect("raced").clone().expect("a racing write");
    let calls = backends.keyvalue.calls.lock().expect("calls").clone();
    let [seeded, raced] = [seeded, raced].map(|revision| Some(Precondition::Matches(revision)));
    assert_eq!(calls, [Call::Get, Call::Set(seeded), Call::Set(raced)]);
    let visits = bucket.get("visits".to_owned()).await.expect("get").map(|entry| entry.value);
    assert_eq!(visits, Some(b"11".to_vec()));
}

// lands a write of `10` before the first conditional set reaches the
// store, as a writer racing the guest would, and records each call
#[derive(Clone, Debug)]
struct Racing {
    inner: KeyValueDefault,
    calls: Arc<Mutex<Vec<Call>>>,
    raced: Arc<Mutex<Option<Revision>>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Call {
    Get,
    Set(Option<Precondition>),
}

impl KeyValueClient for Racing {
    fn open_bucket(&self, identifier: String) -> FutureResult<Arc<dyn Bucket>> {
        let racing = self.clone();
        Box::pin(async move {
            let inner = racing.inner.open_bucket(identifier).await?;
            Ok(Arc::new(Raced { inner, racing }) as Arc<dyn Bucket>)
        })
    }
}

#[derive(Debug)]
struct Raced {
    inner: Arc<dyn Bucket>,
    racing: Racing,
}

impl Bucket for Raced {
    fn get(&self, key: String) -> FutureResult<Option<Entry>> {
        self.racing.calls.lock().expect("calls").push(Call::Get);
        self.inner.get(key)
    }

    fn set(
        &self, key: String, value: Vec<u8>, lifetime: Lifetime, when: Option<Precondition>,
    ) -> FutureResult<Result<Revision, Option<Entry>>> {
        self.racing.calls.lock().expect("calls").push(Call::Set(when.clone()));
        let inner = Arc::clone(&self.inner);
        let raced = Arc::clone(&self.racing.raced);
        Box::pin(async move {
            if when.is_some() && raced.lock().expect("raced").is_none() {
                let racing = inner.set(key.clone(), b"10".to_vec(), Lifetime::Replace(None), None);
                let revision = racing.await?.expect("an unconditional set");
                *raced.lock().expect("raced") = Some(revision);
            }
            inner.set(key, value, lifetime, when).await
        })
    }

    fn delete(
        &self, _key: String, _when: Option<Revision>,
    ) -> FutureResult<Result<(), Option<Entry>>> {
        unreachable!("the scenario deletes nothing")
    }

    fn exists(&self, _key: String) -> FutureResult<bool> {
        unreachable!("the scenario checks no key")
    }

    fn keys(&self) -> FutureResult<Vec<String>> {
        unreachable!("the scenario lists no key")
    }

    fn increment(&self, _key: String, _delta: i64, _ttl: Option<Duration>) -> FutureResult<i64> {
        unreachable!("the scenario counts nothing")
    }
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
