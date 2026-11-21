//! Default in-memory implementation for `omnia:blobstore`
//!
//! This is a lightweight implementation for development use only.

use std::collections::HashMap;
use std::ops::Range;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::Result;
use bytes::Bytes;
use futures::FutureExt;
use omnia_core::Connect;
use parking_lot::RwLock;
use tracing::instrument;

use crate::resource::{Container, FutureResult};
use crate::{BlobStoreClient, ContainerMetadata, ObjectMetadata, Page, missing_container};

/// Default implementation for `omnia:blobstore`.
#[derive(Debug, Clone)]
pub struct BlobStoreDefault {
    store: Arc<RwLock<HashMap<String, InMemContainer>>>,
}

impl Connect for BlobStoreDefault {
    type ConnectOptions = omnia_core::NoOptions;

    #[instrument]
    async fn connect_with(options: Self::ConnectOptions) -> Result<Self> {
        tracing::debug!("initializing in-memory blobstore");
        Ok(Self {
            store: Arc::new(RwLock::new(HashMap::new())),
        })
    }
}

impl BlobStoreClient for BlobStoreDefault {
    fn create_container(&self, name: String) -> FutureResult<Arc<dyn Container>> {
        tracing::trace!("creating container: {name}");
        let store = Arc::clone(&self.store);

        async move {
            // idempotent: re-creating a container returns it rather than replace it
            let container = {
                let mut store = store.write();
                store.entry(name.clone()).or_insert_with(|| InMemContainer::new(name)).clone()
            };
            Ok(Arc::new(container) as Arc<dyn Container>)
        }
        .boxed()
    }

    fn get_container(&self, name: String) -> FutureResult<Arc<dyn Container>> {
        tracing::trace!("getting container: {name}");
        let store = Arc::clone(&self.store);

        async move {
            let container = {
                let store = store.read();
                store.get(&name).cloned().ok_or_else(|| missing_container(&name))?
            };
            Ok(Arc::new(container) as Arc<dyn Container>)
        }
        .boxed()
    }

    fn delete_container(&self, name: String) -> FutureResult<()> {
        tracing::trace!("deleting container: {name}");
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
        tracing::trace!("checking existence of container: {name}");
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
    fn info(&self) -> Result<ContainerMetadata> {
        Ok(ContainerMetadata {
            created_at: self.created_at.duration_since(UNIX_EPOCH).unwrap_or_default().as_secs(),
        })
    }

    fn get_data(&self, name: String, range: Range<u64>) -> FutureResult<Option<Bytes>> {
        tracing::trace!("getting object: {name} from container: {}", self.name);
        let objects = Arc::clone(&self.objects);

        async move {
            let Some(data) = ({
                let objects = objects.read();
                objects.get(&name).map(|object| object.data.clone())
            }) else {
                return Ok(None);
            };

            let len = data.len();
            let clamp = |bound: u64| usize::try_from(bound).map_or(len, |bound| bound.min(len));
            let end = clamp(range.end);
            Ok(Some(data.slice(clamp(range.start).min(end)..end)))
        }
        .boxed()
    }

    fn write_data(&self, name: String, data: Bytes) -> FutureResult<()> {
        tracing::trace!("writing object: {name} to container: {}", self.name);
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

    fn list_objects(&self, prefix: String, after: Option<String>) -> FutureResult<Page> {
        tracing::trace!("listing objects in container: {}", self.name);
        let objects = Arc::clone(&self.objects);

        async move {
            let mut listed: Vec<_> = {
                let objects = objects.read();
                objects
                    .iter()
                    .filter(|(name, _)| {
                        name.starts_with(&prefix)
                            && after.as_ref().is_none_or(|after| *name > after)
                    })
                    .map(|(name, object)| object.metadata(name.clone()))
                    .collect()
            };
            listed.sort_unstable_by(|a, b| a.name.cmp(&b.name));
            Ok(Page {
                objects: listed,
                next: None,
            })
        }
        .boxed()
    }

    fn delete_object(&self, name: String) -> FutureResult<()> {
        tracing::trace!("deleting object: {name} from container: {}", self.name);
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
        tracing::trace!("checking existence of object: {name} in container: {}", self.name);
        let objects = Arc::clone(&self.objects);

        async move {
            let objects = objects.read();
            Ok(objects.contains_key(&name))
        }
        .boxed()
    }

    fn object_info(&self, name: String) -> FutureResult<Option<ObjectMetadata>> {
        tracing::trace!("getting info for object: {name} in container: {}", self.name);
        let objects = Arc::clone(&self.objects);

        async move {
            let objects = objects.read();
            Ok(objects.get(&name).map(|object| object.metadata(name)))
        }
        .boxed()
    }
}

#[derive(Debug, Clone)]
struct Object {
    data: Bytes,
    created_at: SystemTime,
}

impl Object {
    fn metadata(&self, name: String) -> ObjectMetadata {
        ObjectMetadata {
            name,
            created_at: self.created_at.duration_since(UNIX_EPOCH).unwrap_or_default().as_secs(),
            size: self.data.len() as u64,
        }
    }
}
