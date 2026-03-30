//! Default in-memory implementation for wasi-blobstore
//!
//! This is a lightweight implementation for development use only.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::Result;
use futures::FutureExt;
use omnia::Backend;
use parking_lot::RwLock;
use tracing::instrument;

use crate::host::WasiBlobstoreCtx;
use crate::host::generated::wasi::blobstore::container::{ContainerMetadata, ObjectMetadata};
use crate::host::resource::{Container, FutureResult};

/// Options used to connect to the blobstore.
#[derive(Debug, Clone, Default)]
pub struct ConnectOptions;

impl omnia::FromEnv for ConnectOptions {
    fn from_env() -> Result<Self> {
        Ok(Self)
    }
}

/// Default implementation for `wasi:blobstore`.
#[derive(Debug, Clone)]
pub struct BlobstoreDefault {
    store: Arc<RwLock<HashMap<String, InMemContainer>>>,
}

impl Backend for BlobstoreDefault {
    type ConnectOptions = ConnectOptions;

    #[instrument]
    async fn connect_with(options: Self::ConnectOptions) -> Result<Self> {
        tracing::debug!("initializing in-memory blobstore");
        Ok(Self {
            store: Arc::new(RwLock::new(HashMap::new())),
        })
    }
}

impl WasiBlobstoreCtx for BlobstoreDefault {
    fn create_container(&self, name: String) -> FutureResult<Arc<dyn Container>> {
        tracing::debug!("creating container: {name}");
        let store = Arc::clone(&self.store);

        async move {
            let container = InMemContainer::new(name.clone());
            {
                let mut store = store.write();
                store.insert(name, container.clone())
            };
            Ok(Arc::new(container) as Arc<dyn Container>)
        }
        .boxed()
    }

    fn get_container(&self, name: String) -> FutureResult<Arc<dyn Container>> {
        tracing::debug!("getting container: {name}");
        let store = Arc::clone(&self.store);

        async move {
            let container = {
                let store = store.read();
                store
                    .get(&name)
                    .cloned()
                    .ok_or_else(|| wasmtime::Error::msg(format!("container not found: {name}")))?
            };
            Ok(Arc::new(container) as Arc<dyn Container>)
        }
        .boxed()
    }

    fn delete_container(&self, name: String) -> FutureResult<()> {
        tracing::debug!("deleting container: {name}");
        let store = Arc::clone(&self.store);

        async move {
            {
                let mut store = store.write();
                store.remove(&name)
            };
            Ok(())
        }
        .boxed()
    }

    fn container_exists(&self, name: String) -> FutureResult<bool> {
        tracing::debug!("checking existence of container: {name}");
        let store = Arc::clone(&self.store);

        async move {
            let store = store.read();
            Ok(store.contains_key(&name))
        }
        .boxed()
    }
}

#[derive(Debug, Clone)]
struct InMemContainer {
    name: String,
    objects: Arc<RwLock<HashMap<String, Vec<u8>>>>,
    created_at: SystemTime,
}

impl InMemContainer {
    fn new(name: String) -> Self {
        Self {
            name,
            objects: Arc::new(RwLock::new(HashMap::new())),
            created_at: SystemTime::now(),
        }
    }
}

impl Container for InMemContainer {
    fn name(&self) -> Result<String> {
        Ok(self.name.clone())
    }

    fn info(&self) -> Result<ContainerMetadata> {
        let name = self.name.clone();
        let created_at = self.created_at;

        Ok(ContainerMetadata {
            name,
            created_at: created_at.duration_since(UNIX_EPOCH).unwrap_or_default().as_secs(),
        })
    }

    fn get_data(&self, name: String, _start: u64, _end: u64) -> FutureResult<Option<Vec<u8>>> {
        tracing::debug!("getting object: {name} from container: {}", self.name);
        let objects = Arc::clone(&self.objects);

        async move {
            // Note: start/end parameters are ignored in this simple implementation
            // A full implementation would support range reads
            let result = {
                let objects = objects.read();
                objects.get(&name).cloned()
            };
            Ok(result)
        }
        .boxed()
    }

    fn write_data(&self, name: String, data: Vec<u8>) -> FutureResult<()> {
        tracing::debug!("writing object: {name} to container: {}", self.name);
        let objects = Arc::clone(&self.objects);

        async move {
            {
                let mut objects = objects.write();
                objects.insert(name, data)
            };
            Ok(())
        }
        .boxed()
    }

    fn list_objects(&self) -> FutureResult<Vec<String>> {
        tracing::debug!("listing objects in container: {}", self.name);
        let objects = Arc::clone(&self.objects);

        async move {
            let result = {
                let objects = objects.read();
                objects.keys().cloned().collect()
            };
            Ok(result)
        }
        .boxed()
    }

    fn delete_object(&self, name: String) -> FutureResult<()> {
        tracing::debug!("deleting object: {name} from container: {}", self.name);
        let objects = Arc::clone(&self.objects);

        async move {
            {
                let mut objects = objects.write();
                objects.remove(&name)
            };
            Ok(())
        }
        .boxed()
    }

    fn has_object(&self, name: String) -> FutureResult<bool> {
        tracing::debug!("checking existence of object: {name} in container: {}", self.name);
        let objects = Arc::clone(&self.objects);

        async move {
            let objects = objects.read();
            Ok(objects.contains_key(&name))
        }
        .boxed()
    }

    fn object_info(&self, name: String) -> FutureResult<ObjectMetadata> {
        tracing::debug!("getting info for object: {name} in container: {}", self.name);
        let objects = Arc::clone(&self.objects);
        let container_name = self.name.clone();

        async move {
            let size = {
                let objects = objects.read();
                objects
                    .get(&name)
                    .ok_or_else(|| wasmtime::Error::msg(format!("object not found: {name}")))?
                    .len()
            };

            Ok(ObjectMetadata {
                name,
                container: container_name,
                created_at: SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs(),
                size: size as u64,
            })
        }
        .boxed()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn new_ctx() -> BlobstoreDefault {
        BlobstoreDefault::connect_with(ConnectOptions).await.expect("connect")
    }

    #[tokio::test]
    async fn container_crud() {
        let ctx = new_ctx().await;

        ctx.create_container("bucket".to_string()).await.expect("create");
        assert!(ctx.container_exists("bucket".to_string()).await.expect("exists"));

        let retrieved = ctx.get_container("bucket".to_string()).await.expect("get");
        assert_eq!(retrieved.name().expect("name"), "bucket");

        ctx.delete_container("bucket".to_string()).await.expect("delete");
        assert!(!ctx.container_exists("bucket".to_string()).await.expect("exists after delete"));
    }

    #[tokio::test]
    async fn get_nonexistent_container() {
        let ctx = new_ctx().await;
        let result = ctx.get_container("no-such-container".to_string()).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn object_crud() {
        let ctx = new_ctx().await;
        let container = ctx.create_container("obj-crud".to_string()).await.expect("create");

        container.write_data("k1".to_string(), b"v1".to_vec()).await.expect("write");
        let data = container.get_data("k1".to_string(), 0, 0).await.expect("get");
        assert_eq!(data, Some(b"v1".to_vec()));

        assert!(container.has_object("k1".to_string()).await.expect("has k1"));
        assert!(!container.has_object("k2".to_string()).await.expect("has k2"));

        container.write_data("k2".to_string(), b"v2".to_vec()).await.expect("write k2");
        let mut objects = container.list_objects().await.expect("list");
        objects.sort();
        assert_eq!(objects, vec!["k1", "k2"]);

        container.delete_object("k1".to_string()).await.expect("delete k1");
        assert!(!container.has_object("k1".to_string()).await.expect("has k1 after delete"));
    }

    #[tokio::test]
    async fn object_info_valid() {
        let ctx = new_ctx().await;
        let container = ctx.create_container("info-test".to_string()).await.expect("create");

        let payload = b"hello world";
        container.write_data("doc.txt".to_string(), payload.to_vec()).await.expect("write");

        let meta = container.object_info("doc.txt".to_string()).await.expect("object_info");
        assert_eq!(meta.name, "doc.txt");
        assert_eq!(meta.container, "info-test");
        assert_eq!(meta.size, payload.len() as u64);
    }

    #[tokio::test]
    async fn get_nonexistent_object() {
        let ctx = new_ctx().await;
        let container = ctx.create_container("miss".to_string()).await.expect("create");

        let data = container.get_data("ghost".to_string(), 0, 0).await.expect("get");
        assert_eq!(data, None);
    }

    #[tokio::test]
    async fn object_info_nonexistent() {
        let ctx = new_ctx().await;
        let container = ctx.create_container("miss-info".to_string()).await.expect("create");

        let result = container.object_info("ghost".to_string()).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn overwrite_object() {
        let ctx = new_ctx().await;
        let container = ctx.create_container("overwrite".to_string()).await.expect("create");

        container.write_data("key".to_string(), b"first".to_vec()).await.expect("write 1");
        container.write_data("key".to_string(), b"second".to_vec()).await.expect("write 2");

        let data = container.get_data("key".to_string(), 0, 0).await.expect("get");
        assert_eq!(data, Some(b"second".to_vec()));
    }

    #[tokio::test]
    async fn delete_nonexistent_object() {
        let ctx = new_ctx().await;
        let container = ctx.create_container("del-miss".to_string()).await.expect("create");

        container.delete_object("nope".to_string()).await.expect("delete missing should succeed");
    }

    #[tokio::test]
    async fn empty_container_list() {
        let ctx = new_ctx().await;
        let container = ctx.create_container("empty".to_string()).await.expect("create");

        let objects = container.list_objects().await.expect("list");
        assert!(objects.is_empty());
    }

    #[tokio::test]
    async fn create_container_overwrites_existing() {
        let ctx = new_ctx().await;

        let original = ctx.create_container("reused".to_string()).await.expect("create 1");
        original.write_data("stale".to_string(), b"old".to_vec()).await.expect("write");

        let fresh = ctx.create_container("reused".to_string()).await.expect("create 2");
        let objects = fresh.list_objects().await.expect("list");
        assert!(objects.is_empty(), "re-created container should be empty");
    }
}
