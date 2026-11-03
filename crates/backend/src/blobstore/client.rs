//! Service contract for `wasi:blobstore` backends.

use std::fmt::Debug;
use std::sync::Arc;

use anyhow::Result;
use bytes::Bytes;

use super::{ContainerMetadata, ObjectMetadata};
use crate::FutureResult;

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
