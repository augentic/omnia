//! Binary large object storage service contract, value types, and default implementation.

/// Metadata for a blobstore container.
///
/// Mirrors the `container-metadata` record from `wasi:blobstore/types`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContainerMetadata {
    /// The container's name.
    pub name: String,
    /// Seconds since Unix epoch when the container was created.
    pub created_at: u64,
}

/// Metadata for an object in a blobstore container.
///
/// Mirrors the `object-metadata` record from `wasi:blobstore/types`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ObjectMetadata {
    /// The object's name.
    pub name: String,
    /// The object's parent container.
    pub container: String,
    /// Seconds since Unix epoch when the object was created.
    pub created_at: u64,
    /// Size of the object in bytes.
    pub size: u64,
}

#[cfg(not(target_arch = "wasm32"))]
mod host {
    use std::collections::HashMap;
    use std::fmt::Debug;
    use std::sync::Arc;
    use std::time::{SystemTime, UNIX_EPOCH};

    use anyhow::{Result, anyhow};
    use bytes::Bytes;
    use futures::FutureExt as _;
    use parking_lot::RwLock;
    use tracing::instrument;

    use super::{ContainerMetadata, ObjectMetadata};
    use crate::{Connect, FutureResult, NoOptions};

    /// Service contract for `wasi:blobstore` backends.
    pub trait BlobStoreClient: Debug + Send + Sync + 'static {
        /// Create or open a container.
        fn create_container(&self, name: String) -> FutureResult<Arc<dyn Container>>;

        /// Get an existing container.
        fn get_container(&self, name: String) -> FutureResult<Arc<dyn Container>>;

        /// Delete a container and all objects in it.
        fn delete_container(&self, name: String) -> FutureResult<()>;

        /// Check if a container exists.
        fn container_exists(&self, name: String) -> FutureResult<bool>;
    }

    /// Stateful handle to a blobstore container.
    pub trait Container: Debug + Send + Sync + 'static {
        /// The name of the container.
        ///
        /// # Errors
        ///
        /// Returns an error if the container name cannot be retrieved.
        fn name(&self) -> Result<String>;

        /// Returns the metadata for the container.
        ///
        /// # Errors
        ///
        /// Returns an error if the container metadata cannot be retrieved.
        fn info(&self) -> Result<ContainerMetadata>;

        /// Get the byte range `[start..=end]` of object `name` (`end` of `0` or
        /// `u64::MAX` reads to the end); `Ok(None)` when the object is absent.
        fn get_data(&self, name: String, start: u64, end: u64) -> FutureResult<Option<Bytes>>;

        /// Write `data` to object `name`.
        fn write_data(&self, name: String, data: Bytes) -> FutureResult<()>;

        /// List all object names in the container.
        fn list_objects(&self) -> FutureResult<Vec<String>>;

        /// Delete object `name`.
        fn delete_object(&self, name: String) -> FutureResult<()>;

        /// Check if object `name` exists.
        fn has_object(&self, name: String) -> FutureResult<bool>;

        /// Get metadata for object `name`.
        fn object_info(&self, name: String) -> FutureResult<ObjectMetadata>;
    }

    /// Default in-memory implementation of [`BlobStoreClient`].
    #[derive(Debug, Clone, Default)]
    pub struct BlobStoreDefault {
        store: Arc<RwLock<HashMap<String, InMemContainer>>>,
    }

    impl Connect for BlobStoreDefault {
        type ConnectOptions = NoOptions;

        #[instrument]
        async fn connect_with(options: Self::ConnectOptions) -> Result<Self> {
            let _ = options;
            tracing::debug!("initializing in-memory blobstore");
            Ok(Self::default())
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
                    store
                        .get(&name)
                        .cloned()
                        .ok_or_else(|| anyhow!("container not found: {name}"))?
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
            tracing::trace!("getting object: {name} from container: {}", self.name);
            let objects = Arc::clone(&self.objects);

            async move {
                let Some(data) = ({
                    let objects = objects.read();
                    objects.get(&name).map(|object| object.data.clone())
                }) else {
                    return Ok(None);
                };

                // as azure-blob: `end` of 0 or `u64::MAX` reads to the end, else inclusive
                let unbounded = end == 0 || end == u64::MAX;
                if !unbounded && end < start {
                    return Err(anyhow!("invalid byte range: end ({end}) < start ({start})"));
                }
                let len = data.len();
                let clamp = |bound: u64| usize::try_from(bound).map_or(len, |bound| bound.min(len));
                let from = clamp(start);
                let to = if unbounded { len } else { clamp(end.saturating_add(1)) };
                Ok(Some(data.slice(from..to)))
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

        fn list_objects(&self) -> FutureResult<Vec<String>> {
            tracing::trace!("listing objects in container: {}", self.name);
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

        fn object_info(&self, name: String) -> FutureResult<ObjectMetadata> {
            tracing::trace!("getting info for object: {name} in container: {}", self.name);
            let objects = Arc::clone(&self.objects);
            let container_name = self.name.clone();

            async move {
                let guard = objects.read();
                let object = guard.get(&name).ok_or_else(|| anyhow!("object not found: {name}"))?;
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
}

#[cfg(not(target_arch = "wasm32"))]
pub use self::host::{BlobStoreClient, BlobStoreDefault, Container};
