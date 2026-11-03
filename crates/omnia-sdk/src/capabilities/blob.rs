//! Binary large object storage capability.

use std::future::Future;

use anyhow::Result;
pub use omnia_backend::blobstore::{ContainerMetadata, ObjectMetadata};

/// Binary large object storage (WASI Blobstore).
///
/// [`WasiProvider`](crate::WasiProvider) delegates to `wasi:blobstore` via
/// `omnia-wasi-blobstore`.
#[omnia_guest_macros::capability]
pub trait BlobStore: Send + Sync {
    /// Retrieve an object's data from a container.
    fn get(
        &self, container: &str, name: &str,
    ) -> impl Future<Output = Result<Option<Vec<u8>>>> + Send;

    /// Store an object in a container.
    fn put(
        &self, container: &str, name: &str, data: &[u8],
    ) -> impl Future<Output = Result<()>> + Send;

    /// Delete an object from a container.
    fn delete(&self, container: &str, name: &str) -> impl Future<Output = Result<()>> + Send;

    /// List all object names in a container.
    fn list(&self, container: &str) -> impl Future<Output = Result<Vec<String>>> + Send;

    /// Retrieve a byte range of an object's data.
    ///
    /// Both `start` and `end` offsets are inclusive.
    fn get_range(
        &self, container: &str, name: &str, start: u64, end: u64,
    ) -> impl Future<Output = Result<Vec<u8>>> + Send;

    /// Return metadata for an object.
    fn object_info(
        &self, container: &str, name: &str,
    ) -> impl Future<Output = Result<ObjectMetadata>> + Send;

    /// Create a new empty container.
    fn create_container(&self, name: &str) -> impl Future<Output = Result<()>> + Send;

    /// Delete a container and all objects within it.
    fn delete_container(&self, name: &str) -> impl Future<Output = Result<()>> + Send;

    /// Check whether a container exists.
    fn container_exists(&self, name: &str) -> impl Future<Output = Result<bool>> + Send;

    /// Return metadata for a container.
    fn container_info(
        &self, container: &str,
    ) -> impl Future<Output = Result<ContainerMetadata>> + Send;

    /// Check whether an object exists in a container.
    fn has(&self, container: &str, name: &str) -> impl Future<Output = Result<bool>> + Send {
        async move { Ok(self.get(container, name).await?.is_some()) }
    }

    /// Delete multiple objects from a container.
    fn delete_objects(
        &self, container: &str, names: &[String],
    ) -> impl Future<Output = Result<()>> + Send {
        async move {
            for name in names {
                self.delete(container, name).await?;
            }
            Ok(())
        }
    }

    /// Remove all objects from a container, leaving it empty.
    fn clear(&self, container: &str) -> impl Future<Output = Result<()>> + Send {
        async move {
            for name in self.list(container).await? {
                self.delete(container, &name).await?;
            }
            Ok(())
        }
    }

    /// Copy an object to the same or a different container.
    ///
    /// Overwrites the destination object if it already exists; fails if the
    /// source object is missing, or if `put` refuses the destination
    /// container.
    fn copy_object(
        &self, src_container: &str, src_name: &str, dest_container: &str, dest_name: &str,
    ) -> impl Future<Output = Result<()>> + Send {
        async move {
            let data = self.get(src_container, src_name).await?.ok_or_else(|| {
                anyhow::anyhow!("source object not found: {src_container}/{src_name}")
            })?;
            self.put(dest_container, dest_name, &data).await
        }
    }

    /// Move or rename an object to the same or a different container.
    ///
    /// Overwrites the destination object if it already exists; fails as
    /// [`copy_object`](Self::copy_object) does, leaving the source in place.
    fn move_object(
        &self, src_container: &str, src_name: &str, dest_container: &str, dest_name: &str,
    ) -> impl Future<Output = Result<()>> + Send {
        async move {
            if src_container == dest_container && src_name == dest_name {
                return Ok(());
            }
            self.copy_object(src_container, src_name, dest_container, dest_name).await?;
            self.delete(src_container, src_name).await
        }
    }
}

#[cfg(all(target_arch = "wasm32", target_os = "wasi"))]
impl BlobStore for crate::WasiProvider {
    async fn get(&self, container: &str, name: &str) -> Result<Option<Vec<u8>>> {
        use anyhow::anyhow;
        use omnia_wasi_blobstore::types::IncomingValue;

        let ctr = open_container(container).await?;

        // one round trip: a not-found error is `None`, rather than racing an existence check
        let incoming = match ctr.get_data(name.to_string(), 0, u64::MAX).await {
            Ok(incoming) => incoming,
            // wasi-blobstore lowers a missing object to this message for every backend
            Err(e) if e.contains("object not found") => return Ok(None),
            Err(e) => return Err(anyhow!("reading object: {e}")),
        };
        let data = IncomingValue::incoming_value_consume_sync(incoming)
            .map_err(|e| anyhow!("consuming incoming value: {e}"))?;
        Ok(Some(data))
    }

    async fn put(&self, container: &str, name: &str, data: &[u8]) -> Result<()> {
        use anyhow::anyhow;
        use omnia_wasi_blobstore::types::OutgoingValue;

        // the wasi:io write budget: `blocking-write-and-flush` takes at most 4096 bytes
        const WRITE_CHUNK: usize = 4096;

        let ctr = open_container(container).await?;
        let outgoing = OutgoingValue::new_outgoing_value();
        {
            let body = outgoing
                .outgoing_value_write_body()
                .await
                .map_err(|e| anyhow!("getting write body: {e:?}"))?;
            for chunk in data.chunks(WRITE_CHUNK) {
                body.blocking_write_and_flush(chunk).map_err(|e| anyhow!("writing data: {e}"))?;
            }
        };
        ctr.write_data(name.to_string(), &outgoing)
            .await
            .map_err(|e| anyhow!("writing object: {e}"))?;
        OutgoingValue::finish(outgoing).map_err(|e| anyhow!("finishing write: {e}"))?;
        Ok(())
    }

    async fn delete(&self, container: &str, name: &str) -> Result<()> {
        use anyhow::anyhow;

        let ctr = open_container(container).await?;
        ctr.delete_object(name.to_string()).await.map_err(|e| anyhow!("deleting object: {e}"))
    }

    async fn list(&self, container: &str) -> Result<Vec<String>> {
        use anyhow::anyhow;

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

    async fn get_range(
        &self, container: &str, name: &str, start: u64, end: u64,
    ) -> Result<Vec<u8>> {
        use anyhow::anyhow;
        use omnia_wasi_blobstore::types::IncomingValue;

        let ctr = open_container(container).await?;
        let incoming = ctr
            .get_data(name.to_string(), start, end)
            .await
            .map_err(|e| anyhow!("reading object range: {e}"))?;
        let data = IncomingValue::incoming_value_consume_sync(incoming)
            .map_err(|e| anyhow!("consuming incoming value: {e}"))?;
        Ok(data)
    }

    async fn object_info(&self, container: &str, name: &str) -> Result<ObjectMetadata> {
        use anyhow::anyhow;

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

    async fn create_container(&self, name: &str) -> Result<()> {
        use anyhow::anyhow;

        omnia_wasi_blobstore::blobstore::create_container(name.to_string())
            .await
            .map_err(|e| anyhow!("creating container: {e}"))?;
        Ok(())
    }

    async fn delete_container(&self, name: &str) -> Result<()> {
        use anyhow::anyhow;

        omnia_wasi_blobstore::blobstore::delete_container(name.to_string())
            .await
            .map_err(|e| anyhow!("deleting container: {e}"))
    }

    async fn container_exists(&self, name: &str) -> Result<bool> {
        use anyhow::anyhow;

        omnia_wasi_blobstore::blobstore::container_exists(name.to_string())
            .await
            .map_err(|e| anyhow!("checking container existence: {e}"))
    }

    async fn container_info(&self, container: &str) -> Result<ContainerMetadata> {
        use anyhow::anyhow;

        let ctr = open_container(container).await?;
        let info = ctr.info().map_err(|e| anyhow!("getting container info: {e}"))?;
        Ok(ContainerMetadata {
            name: info.name,
            created_at: info.created_at,
        })
    }

    async fn has(&self, container: &str, name: &str) -> Result<bool> {
        use anyhow::anyhow;

        let ctr = open_container(container).await?;
        ctr.has_object(name.to_string())
            .await
            .map_err(|e| anyhow!("checking object existence: {e}"))
    }

    async fn delete_objects(&self, container: &str, names: &[String]) -> Result<()> {
        use anyhow::anyhow;

        let names = names.to_vec();
        let ctr = open_container(container).await?;
        ctr.delete_objects(names).await.map_err(|e| anyhow!("deleting objects: {e}"))
    }

    async fn clear(&self, container: &str) -> Result<()> {
        use anyhow::anyhow;

        let ctr = open_container(container).await?;
        ctr.clear().await.map_err(|e| anyhow!("clearing container: {e}"))
    }

    async fn copy_object(
        &self, src_container: &str, src_name: &str, dest_container: &str, dest_name: &str,
    ) -> Result<()> {
        use anyhow::anyhow;
        use omnia_wasi_blobstore::types::ObjectId;

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

    async fn move_object(
        &self, src_container: &str, src_name: &str, dest_container: &str, dest_name: &str,
    ) -> Result<()> {
        use anyhow::anyhow;
        use omnia_wasi_blobstore::types::ObjectId;

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

#[cfg(all(target_arch = "wasm32", target_os = "wasi"))]
async fn open_container(container: &str) -> Result<omnia_wasi_blobstore::container::Container> {
    use anyhow::anyhow;

    omnia_wasi_blobstore::blobstore::get_container(container.to_string())
        .await
        .map_err(|e| anyhow!("opening container: {e}"))
}
