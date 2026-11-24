//! End-to-end tests for `omnia:blobstore`: every scenario runs a real guest
//! component from `crates/test-programs` through the omnia runtime against
//! the in-memory default. The guest asserts what it observes across the
//! boundary (and traps on failure); the host side seeds a container and
//! asserts what persisted after the run.

#![cfg(not(target_arch = "wasm32"))]

use std::ops::Range;
use std::sync::Arc;

use omnia::ExitStatus;
use omnia_test::host::{Backends, Deployment};
use omnia_wasi_blobstore::{
    BlobStoreClient, BlobStoreDefault, Bytes, Container, ContainerMetadata, FutureResult,
    ObjectMetadata, Page, WasiBlobStore,
};

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

#[tokio::test]
async fn blobstore_unstable() {
    let backends = Backends::defaults().await;
    backends.blobstore.create_container("objects".to_owned()).await.expect("objects");

    run_guest(test_programs::BLOBSTORE_UNSTABLE, backends.clone()).await;

    assert_eq!(backends.object("objects", "big").await, None, "a refused call stored nothing");
}

#[tokio::test]
async fn blobstore_deferred_write() {
    let defaults = Backends::defaults().await;
    let inner = defaults.blobstore.clone();
    let backends = defaults.blobstore(Deferred(inner));
    backends.blobstore.create_container("objects".to_owned()).await.expect("objects");

    let status = Deployment::new()
        .guest("guest", test_programs::BLOBSTORE_CONTAINER)
        .run_host::<WasiBlobStore, _>(backends.clone())
        .await
        .expect("guest runs");
    assert_eq!(status, ExitStatus::SUCCESS, "guest failed");

    assert_eq!(backends.object("objects", "digits").await, Some(b"0123456789".to_vec()));
}

// yields once in `write_data` so every `put` is still pending after its first
// poll
#[derive(Clone, Debug)]
struct Deferred(BlobStoreDefault);

impl BlobStoreClient for Deferred {
    fn create_container(&self, name: String) -> FutureResult<Arc<dyn Container>> {
        let inner = self.0.create_container(name);
        Box::pin(async move { Ok(Arc::new(DeferredContainer(inner.await?)) as Arc<dyn Container>) })
    }

    fn get_container(&self, name: String) -> FutureResult<Arc<dyn Container>> {
        let inner = self.0.get_container(name);
        Box::pin(async move { Ok(Arc::new(DeferredContainer(inner.await?)) as Arc<dyn Container>) })
    }

    fn delete_container(&self, name: String) -> FutureResult<()> {
        self.0.delete_container(name)
    }

    fn container_exists(&self, name: String) -> FutureResult<bool> {
        self.0.container_exists(name)
    }
}

#[derive(Debug)]
struct DeferredContainer(Arc<dyn Container>);

impl Container for DeferredContainer {
    fn info(&self) -> anyhow::Result<ContainerMetadata> {
        self.0.info()
    }

    fn get_data(&self, name: String, range: Range<u64>) -> FutureResult<Option<Bytes>> {
        self.0.get_data(name, range)
    }

    fn write_data(&self, name: String, data: Bytes) -> FutureResult<()> {
        let inner = Arc::clone(&self.0);
        Box::pin(async move {
            tokio::task::yield_now().await;
            inner.write_data(name, data).await
        })
    }

    fn list_objects(&self, prefix: String, after: Option<String>) -> FutureResult<Page> {
        self.0.list_objects(prefix, after)
    }

    fn delete_object(&self, name: String) -> FutureResult<()> {
        self.0.delete_object(name)
    }

    fn has_object(&self, name: String) -> FutureResult<bool> {
        self.0.has_object(name)
    }

    fn object_info(&self, name: String) -> FutureResult<Option<ObjectMetadata>> {
        self.0.object_info(name)
    }
}
