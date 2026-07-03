//! Binary large object storage capability.

use std::future::Future;

use anyhow::Result;

/// Metadata for a blobstore container.
///
/// Mirrors the `container-metadata` record from `wasi:blobstore/types`.
#[derive(Clone, Debug)]
pub struct ContainerMetadata {
    /// The container's name.
    pub name: String,
    /// Seconds since Unix epoch when the container was created.
    pub created_at: u64,
}

/// Metadata for an object in a blobstore container.
///
/// Mirrors the `object-metadata` record from `wasi:blobstore/types`.
#[derive(Clone, Debug)]
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

/// Binary large object storage (WASI Blobstore).
///
/// Default WASM implementations delegate to `wasi:blobstore` via
/// `omnia-wasi-blobstore`.
pub trait BlobStore: Send + Sync {
    /// Retrieve an object's data from a container.
    #[cfg(not(target_arch = "wasm32"))]
    fn get(
        &self, container: &str, name: &str,
    ) -> impl Future<Output = Result<Option<Vec<u8>>>> + Send;

    /// Store an object in a container.
    #[cfg(not(target_arch = "wasm32"))]
    fn put(
        &self, container: &str, name: &str, data: &[u8],
    ) -> impl Future<Output = Result<()>> + Send;

    /// Delete an object from a container.
    #[cfg(not(target_arch = "wasm32"))]
    fn delete(&self, container: &str, name: &str) -> impl Future<Output = Result<()>> + Send;

    /// Check whether an object exists in a container.
    #[cfg(not(target_arch = "wasm32"))]
    fn has(&self, container: &str, name: &str) -> impl Future<Output = Result<bool>> + Send;

    /// List all object names in a container.
    #[cfg(not(target_arch = "wasm32"))]
    fn list(&self, container: &str) -> impl Future<Output = Result<Vec<String>>> + Send;

    /// Retrieve a byte range of an object's data.
    ///
    /// Both `start` and `end` offsets are inclusive.
    #[cfg(not(target_arch = "wasm32"))]
    fn get_range(
        &self, container: &str, name: &str, start: u64, end: u64,
    ) -> impl Future<Output = Result<Vec<u8>>> + Send;

    /// Return metadata for an object.
    #[cfg(not(target_arch = "wasm32"))]
    fn object_info(
        &self, container: &str, name: &str,
    ) -> impl Future<Output = Result<ObjectMetadata>> + Send;

    /// Delete multiple objects from a container.
    #[cfg(not(target_arch = "wasm32"))]
    fn delete_objects(
        &self, container: &str, names: &[String],
    ) -> impl Future<Output = Result<()>> + Send;

    /// Remove all objects from a container, leaving it empty.
    #[cfg(not(target_arch = "wasm32"))]
    fn clear(&self, container: &str) -> impl Future<Output = Result<()>> + Send;

    /// Create a new empty container.
    #[cfg(not(target_arch = "wasm32"))]
    fn create_container(&self, name: &str) -> impl Future<Output = Result<()>> + Send;

    /// Delete a container and all objects within it.
    #[cfg(not(target_arch = "wasm32"))]
    fn delete_container(&self, name: &str) -> impl Future<Output = Result<()>> + Send;

    /// Check whether a container exists.
    #[cfg(not(target_arch = "wasm32"))]
    fn container_exists(&self, name: &str) -> impl Future<Output = Result<bool>> + Send;

    /// Return metadata for a container.
    #[cfg(not(target_arch = "wasm32"))]
    fn container_info(
        &self, container: &str,
    ) -> impl Future<Output = Result<ContainerMetadata>> + Send;

    /// Copy an object to the same or a different container.
    ///
    /// Overwrites the destination object if it already exists. Returns an
    /// error if the destination container does not exist.
    #[cfg(not(target_arch = "wasm32"))]
    fn copy_object(
        &self, src_container: &str, src_name: &str, dest_container: &str, dest_name: &str,
    ) -> impl Future<Output = Result<()>> + Send;

    /// Move or rename an object to the same or a different container.
    ///
    /// Overwrites the destination object if it already exists. Returns an
    /// error if the destination container does not exist.
    #[cfg(not(target_arch = "wasm32"))]
    fn move_object(
        &self, src_container: &str, src_name: &str, dest_container: &str, dest_name: &str,
    ) -> impl Future<Output = Result<()>> + Send;

    /// Retrieve an object's data from a container.
    #[cfg(target_arch = "wasm32")]
    fn get(
        &self, container: &str, name: &str,
    ) -> impl Future<Output = Result<Option<Vec<u8>>>> + Send {
        use anyhow::anyhow;
        use omnia_wasi_blobstore::types::IncomingValue;

        async move {
            let ctr = open_container(container).await?;
            if !ctr
                .has_object(name.to_string())
                .await
                .map_err(|e| anyhow!("checking object existence: {e}"))?
            {
                return Ok(None);
            }
            let incoming = ctr
                .get_data(name.to_string(), 0, u64::MAX)
                .await
                .map_err(|e| anyhow!("reading object: {e}"))?;
            let data = IncomingValue::incoming_value_consume_sync(incoming)
                .map_err(|e| anyhow!("consuming incoming value: {e}"))?;
            Ok(Some(data))
        }
    }

    /// Store an object in a container.
    #[cfg(target_arch = "wasm32")]
    fn put(
        &self, container: &str, name: &str, data: &[u8],
    ) -> impl Future<Output = Result<()>> + Send {
        use anyhow::anyhow;
        use omnia_wasi_blobstore::types::OutgoingValue;

        async move {
            let ctr = open_container(container).await?;
            let outgoing = OutgoingValue::new_outgoing_value();
            {
                let body = outgoing
                    .outgoing_value_write_body()
                    .await
                    .map_err(|e| anyhow!("getting write body: {e:?}"))?;
                body.blocking_write_and_flush(data).map_err(|e| anyhow!("writing data: {e}"))?;
            };
            ctr.write_data(name.to_string(), &outgoing)
                .await
                .map_err(|e| anyhow!("writing object: {e}"))?;
            OutgoingValue::finish(outgoing).map_err(|e| anyhow!("finishing write: {e}"))?;
            Ok(())
        }
    }

    /// Delete an object from a container.
    #[cfg(target_arch = "wasm32")]
    fn delete(&self, container: &str, name: &str) -> impl Future<Output = Result<()>> + Send {
        use anyhow::anyhow;

        async move {
            let ctr = open_container(container).await?;
            ctr.delete_object(name.to_string()).await.map_err(|e| anyhow!("deleting object: {e}"))
        }
    }

    /// Check whether an object exists in a container.
    #[cfg(target_arch = "wasm32")]
    fn has(&self, container: &str, name: &str) -> impl Future<Output = Result<bool>> + Send {
        use anyhow::anyhow;

        async move {
            let ctr = open_container(container).await?;
            ctr.has_object(name.to_string())
                .await
                .map_err(|e| anyhow!("checking object existence: {e}"))
        }
    }

    /// List all object names in a container.
    #[cfg(target_arch = "wasm32")]
    fn list(&self, container: &str) -> impl Future<Output = Result<Vec<String>>> + Send {
        use anyhow::anyhow;

        async move {
            let ctr = open_container(container).await?;
            let stream = ctr.list_objects().await.map_err(|e| anyhow!("listing objects: {e}"))?;
            let mut names = Vec::new();
            loop {
                let (batch, done) = stream
                    .read_stream_object_names(100)
                    .await
                    .map_err(|e| anyhow!("reading object names: {e}"))?;
                names.extend(batch);
                if done {
                    break;
                }
            }
            Ok(names)
        }
    }

    /// Retrieve a byte range of an object's data.
    ///
    /// Both `start` and `end` offsets are inclusive.
    #[cfg(target_arch = "wasm32")]
    fn get_range(
        &self, container: &str, name: &str, start: u64, end: u64,
    ) -> impl Future<Output = Result<Vec<u8>>> + Send {
        use anyhow::anyhow;
        use omnia_wasi_blobstore::types::IncomingValue;

        async move {
            let ctr = open_container(container).await?;
            let incoming = ctr
                .get_data(name.to_string(), start, end)
                .await
                .map_err(|e| anyhow!("reading object range: {e}"))?;
            let data = IncomingValue::incoming_value_consume_sync(incoming)
                .map_err(|e| anyhow!("consuming incoming value: {e}"))?;
            Ok(data)
        }
    }

    /// Return metadata for an object.
    #[cfg(target_arch = "wasm32")]
    fn object_info(
        &self, container: &str, name: &str,
    ) -> impl Future<Output = Result<ObjectMetadata>> + Send {
        use anyhow::anyhow;

        async move {
            let ctr = open_container(container).await?;
            let info = ctr
                .object_info(name.to_string())
                .await
                .map_err(|e| anyhow!("getting object info: {e}"))?;
            Ok(ObjectMetadata {
                name: info.name,
                container: info.container,
                created_at: info.created_at,
                size: info.size,
            })
        }
    }

    /// Delete multiple objects from a container.
    #[cfg(target_arch = "wasm32")]
    fn delete_objects(
        &self, container: &str, names: &[String],
    ) -> impl Future<Output = Result<()>> + Send {
        use anyhow::anyhow;

        let names = names.to_vec();
        async move {
            let ctr = open_container(container).await?;
            ctr.delete_objects(names).await.map_err(|e| anyhow!("deleting objects: {e}"))
        }
    }

    /// Remove all objects from a container, leaving it empty.
    #[cfg(target_arch = "wasm32")]
    fn clear(&self, container: &str) -> impl Future<Output = Result<()>> + Send {
        use anyhow::anyhow;

        async move {
            let ctr = open_container(container).await?;
            ctr.clear().await.map_err(|e| anyhow!("clearing container: {e}"))
        }
    }

    /// Create a new empty container.
    #[cfg(target_arch = "wasm32")]
    fn create_container(&self, name: &str) -> impl Future<Output = Result<()>> + Send {
        use anyhow::anyhow;

        async move {
            omnia_wasi_blobstore::blobstore::create_container(name.to_string())
                .await
                .map_err(|e| anyhow!("creating container: {e}"))?;
            Ok(())
        }
    }

    /// Delete a container and all objects within it.
    #[cfg(target_arch = "wasm32")]
    fn delete_container(&self, name: &str) -> impl Future<Output = Result<()>> + Send {
        use anyhow::anyhow;

        async move {
            omnia_wasi_blobstore::blobstore::delete_container(name.to_string())
                .await
                .map_err(|e| anyhow!("deleting container: {e}"))
        }
    }

    /// Check whether a container exists.
    #[cfg(target_arch = "wasm32")]
    fn container_exists(&self, name: &str) -> impl Future<Output = Result<bool>> + Send {
        use anyhow::anyhow;

        async move {
            omnia_wasi_blobstore::blobstore::container_exists(name.to_string())
                .await
                .map_err(|e| anyhow!("checking container existence: {e}"))
        }
    }

    /// Return metadata for a container.
    #[cfg(target_arch = "wasm32")]
    fn container_info(
        &self, container: &str,
    ) -> impl Future<Output = Result<ContainerMetadata>> + Send {
        use anyhow::anyhow;

        async move {
            let ctr = open_container(container).await?;
            let info = ctr.info().map_err(|e| anyhow!("getting container info: {e}"))?;
            Ok(ContainerMetadata {
                name: info.name,
                created_at: info.created_at,
            })
        }
    }

    /// Copy an object to the same or a different container.
    ///
    /// Overwrites the destination object if it already exists. Returns an
    /// error if the destination container does not exist.
    #[cfg(target_arch = "wasm32")]
    fn copy_object(
        &self, src_container: &str, src_name: &str, dest_container: &str, dest_name: &str,
    ) -> impl Future<Output = Result<()>> + Send {
        use anyhow::anyhow;
        use omnia_wasi_blobstore::types::ObjectId;

        async move {
            let src = ObjectId {
                container: src_container.to_string(),
                object: src_name.to_string(),
            };
            let dest = ObjectId {
                container: dest_container.to_string(),
                object: dest_name.to_string(),
            };
            omnia_wasi_blobstore::blobstore::copy_object(src, dest)
                .await
                .map_err(|e| anyhow!("copying object: {e}"))
        }
    }

    /// Move or rename an object to the same or a different container.
    ///
    /// Overwrites the destination object if it already exists. Returns an
    /// error if the destination container does not exist.
    #[cfg(target_arch = "wasm32")]
    fn move_object(
        &self, src_container: &str, src_name: &str, dest_container: &str, dest_name: &str,
    ) -> impl Future<Output = Result<()>> + Send {
        use anyhow::anyhow;
        use omnia_wasi_blobstore::types::ObjectId;

        async move {
            let src = ObjectId {
                container: src_container.to_string(),
                object: src_name.to_string(),
            };
            let dest = ObjectId {
                container: dest_container.to_string(),
                object: dest_name.to_string(),
            };
            omnia_wasi_blobstore::blobstore::move_object(src, dest)
                .await
                .map_err(|e| anyhow!("moving object: {e}"))
        }
    }
}

/// Open a blobstore container, mapping the WIT error into `anyhow`.
#[cfg(target_arch = "wasm32")]
async fn open_container(container: &str) -> Result<omnia_wasi_blobstore::container::Container> {
    use anyhow::anyhow;

    omnia_wasi_blobstore::blobstore::get_container(container.to_string())
        .await
        .map_err(|e| anyhow!("opening container: {e}"))
}
