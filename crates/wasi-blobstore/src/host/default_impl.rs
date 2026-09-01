//! Default in-memory implementation for wasi-blobstore
//!
//! This is a lightweight implementation for development use only.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Result, anyhow};
use bytes::Bytes;
use futures::FutureExt;
use omnia_core::Backend;
use parking_lot::RwLock;
use tracing::instrument;

use crate::host::WasiBlobstoreCtx;
use crate::host::generated::wasi::blobstore::container::{ContainerMetadata, ObjectMetadata};
use crate::host::resource::{Container, FutureResult};

/// Default implementation for `wasi:blobstore`.
#[derive(Debug, Clone)]
pub struct BlobstoreDefault {
    store: Arc<RwLock<HashMap<String, InMemContainer>>>,
}

impl Backend for BlobstoreDefault {
    type ConnectOptions = omnia_core::NoOptions;

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
            // Idempotent: re-creating an existing container returns it
            // rather than silently replacing it (and destroying its objects).
            let container = {
                let mut store = store.write();
                store.entry(name.clone()).or_insert_with(|| InMemContainer::new(name)).clone()
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
    objects: Arc<RwLock<HashMap<String, Object>>>,
    created_at: SystemTime,
}

#[derive(Debug, Clone)]
struct Object {
    data: Bytes,
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

    fn get_data(&self, name: String, start: u64, end: u64) -> FutureResult<Option<Bytes>> {
        tracing::debug!("getting object: {name} from container: {}", self.name);
        let objects = Arc::clone(&self.objects);

        async move {
            let Some(data) = ({
                let objects = objects.read();
                objects.get(&name).map(|object| object.data.clone())
            }) else {
                return Ok(None);
            };

            // Range semantics match the production backends (azure-blob):
            // `end` of 0 or `u64::MAX` reads to the end; otherwise `end` is
            // inclusive per the WIT contract, clamped to the object's length.
            let unbounded = end == 0 || end == u64::MAX;
            if !unbounded && end < start {
                return Err(anyhow!("invalid byte range: end ({end}) < start ({start})"));
            }
            let len = data.len() as u64;
            let from = start.min(len);
            let to = if unbounded { len } else { end.saturating_add(1).min(len) };
            // Both bounds are clamped to the object's length, itself a usize.
            #[allow(clippy::cast_possible_truncation)]
            let range = from as usize..to as usize;
            Ok(Some(data.slice(range)))
        }
        .boxed()
    }

    fn write_data(&self, name: String, data: Bytes) -> FutureResult<()> {
        tracing::debug!("writing object: {name} to container: {}", self.name);
        let objects = Arc::clone(&self.objects);

        async move {
            {
                let mut objects = objects.write();
                objects.insert(
                    name,
                    Object {
                        data,
                        created_at: SystemTime::now(),
                    },
                )
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
            let guard = objects.read();
            let object = guard
                .get(&name)
                .ok_or_else(|| wasmtime::Error::msg(format!("object not found: {name}")))?;
            let (size, created_at) = (object.data.len(), object.created_at);
            drop(guard);

            Ok(ObjectMetadata {
                name,
                container: container_name,
                created_at: created_at.duration_since(UNIX_EPOCH).unwrap_or_default().as_secs(),
                size: size as u64,
            })
        }
        .boxed()
    }
}
