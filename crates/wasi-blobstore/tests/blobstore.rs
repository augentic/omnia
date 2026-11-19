//! End-to-end tests for `omnia:blobstore`: every scenario runs a real guest
//! component from `crates/test-programs` through the omnia runtime against
//! the in-memory default. The guest asserts what it observes across the
//! boundary (and traps on failure); the host side seeds a container and
//! asserts what persisted after the run.

#![cfg(not(target_arch = "wasm32"))]

use omnia::ExitStatus;
use omnia_test::host::{Backends, Deployment};
use omnia_wasi_blobstore::{BlobStoreClient as _, WasiBlobStore};

// Every guest program in `crates/test-programs` must have a matching test
// here; a new program without one fails to compile.
test_programs::foreach_blobstore!();

async fn run_guest(wasm: &str, backends: Backends) {
    let status = Deployment::new()
        .guest("guest", wasm)
        .run_host::<WasiBlobStore, _>(backends)
        .await
        .expect("guest runs");
    assert_eq!(status, ExitStatus::SUCCESS, "guest `{wasm}` failed");
}

#[tokio::test]
async fn blobstore_blob_store() {
    let backends = Backends::defaults().await;
    let seeded = backends.blobstore.create_container("seeded".to_owned()).await.expect("seeded");
    seeded.write_data("from-host".to_owned(), b"hello".to_vec().into()).await.expect("seed");

    run_guest(test_programs::BLOBSTORE_BLOB_STORE, backends.clone()).await;

    // the guest's bytes are stored as they are
    assert_eq!(backends.object("contract-host", "raw").await, Some(b"raw".to_vec()));
}

#[tokio::test]
async fn blobstore_container() {
    let backends = Backends::defaults().await;
    backends.blobstore.create_container("objects".to_owned()).await.expect("objects");

    run_guest(test_programs::BLOBSTORE_CONTAINER, backends.clone()).await;

    // a write the guest abandoned or cut short stored nothing
    assert_eq!(backends.object("objects", "digits").await, Some(b"0123456789".to_vec()));
    assert_eq!(backends.object("objects", "abandoned").await, None);
    assert_eq!(backends.object("objects", "short").await, None);
}
