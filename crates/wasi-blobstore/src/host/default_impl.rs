//! Default in-memory implementation for wasi-blobstore
//!
//! This is a lightweight implementation for development use only.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Result, anyhow};
use futures::FutureExt;
use parking_lot::RwLock;
use tracing::instrument;
use yetti::Backend;

use crate::host::WasiBlobstoreCtx;
use crate::host::generated::wasi::blobstore::container::{ContainerMetadata, ObjectMetadata};
use crate::host::resource::{Container, FutureResult};

#[derive(Debug, Clone, Default)]
pub struct ConnectOptions;

impl yetti::FromEnv for ConnectOptions {
    fn from_env() -> Result<Self> {
        Ok(Self)
    }
}

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
                store.get(&name).cloned().ok_or_else(|| anyhow!("container not found: {name}"))?
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
    fn name(&self) -> anyhow::Result<String> {
        Ok(self.name.clone())
    }

    fn info(&self) -> anyhow::Result<ContainerMetadata> {
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
                objects.get(&name).ok_or_else(|| anyhow!("object not found: {name}"))?.len()
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

    #[tokio::test]
    async fn container_operations() {
        let ctx = BlobstoreDefault::connect_with(ConnectOptions).await.expect("connect");

        // Test create and get container
        let container =
            ctx.create_container("test-container".to_string()).await.expect("create container");

        // Test write and read data
        container.write_data("object1".to_string(), b"data1".to_vec()).await.expect("write data");

        let data = container.get_data("object1".to_string(), 0, 0).await.expect("get data");
        assert_eq!(data, Some(b"data1".to_vec()));

        // Test object existence
        assert!(container.has_object("object1".to_string()).await.expect("has object"));
        assert!(!container.has_object("object2".to_string()).await.expect("has object"));

        // Test list objects
        container.write_data("object2".to_string(), b"data2".to_vec()).await.expect("write data");
        let mut objects = container.list_objects().await.expect("list objects");
        objects.sort();
        assert_eq!(objects, vec!["object1".to_string(), "object2".to_string()]);

        // Test delete object
        container.delete_object("object1".to_string()).await.expect("delete object");
        assert!(!container.has_object("object1".to_string()).await.expect("has object"));

        // Test container metadata
        let info = container.info().expect("container info");
        assert_eq!(info.name, "test-container");
    }
}
