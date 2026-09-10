//! End-to-end tests for `wasi:blobstore`: every scenario runs a real guest
//! component from `crates/test-programs` through the omnia runtime against
//! the in-memory default. The guest asserts what it observes across the
//! boundary (and traps on failure); the host side seeds a container and
//! asserts what persisted after the run.

#![cfg(not(target_arch = "wasm32"))]

use omnia::ExitStatus;
use omnia_test::host::{Backends, Deployment};
use omnia_wasi_blobstore::{WasiBlobstore, WasiBlobstoreCtx as _};

// Every guest program in `crates/test-programs` must have a matching test
// here; a new program without one fails to compile.
test_programs::foreach_blobstore!();

/// Run one guest program against `backends`, requiring a clean exit.
async fn run_guest(wasm: &str, backends: Backends) {
    let status = Deployment::new()
        .guest("guest", wasm)
        .run_host::<WasiBlobstore, _>(backends)
        .await
        .expect("guest runs");
    assert_eq!(status, ExitStatus::SUCCESS, "guest `{wasm}` failed");
}

#[tokio::test]
async fn blobstore_objects() {
    let backends = Backends::defaults().await;
    let seeded = backends.blobstore.create_container("seeded".to_owned()).await.expect("seeded");
    seeded.write_data("from-host".to_owned(), b"hello".to_vec().into()).await.expect("seed");

    run_guest(test_programs::BLOBSTORE_OBJECTS, backends.clone()).await;

    // Persisted state: the guest's reply landed beside the seed, and the
    // container it deleted is gone with everything in it.
    assert_eq!(backends.object("seeded", "from-host").await, Some(b"hello".to_vec()));
    assert_eq!(backends.object("seeded", "reply").await, Some(b"from-guest".to_vec()));
    assert!(!backends.blobstore.container_exists("objects".to_owned()).await.expect("exists"));
    assert_eq!(backends.object("objects", "doomed").await, None);
}

#[tokio::test]
async fn blobstore_copy_move() {
    let backends = Backends::defaults().await;

    run_guest(test_programs::BLOBSTORE_COPY_MOVE, backends.clone()).await;

    // Persisted state: the copies and the move landed, the moved source is
    // gone, and the refused destination container was never created.
    assert_eq!(backends.object("a", "doc").await, Some(b"payload".to_vec()));
    assert_eq!(backends.object("b", "doc").await, None);
    assert_eq!(backends.object("b", "doc2").await, Some(b"payload".to_vec()));
    assert_eq!(backends.object("b", "x").await, None);
    assert_eq!(backends.object("c", "doc").await, Some(b"payload".to_vec()));
    assert!(!backends.blobstore.container_exists("nowhere".to_owned()).await.expect("exists"));
}
