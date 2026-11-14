//! Binary large object storage capability.

use std::future::Future;
use std::ops::Range;
use std::time::SystemTime;

use bytes::Bytes;

use crate::capability::{Error, ErrorKind, Page, Result};

/// Store and retrieve binary objects in a deployment's containers.
///
/// An absent object is `Ok(None)`, and a call on an object in a container
/// that does not exist fails with [`ErrorKind::NotFound`].
///
/// Every provider accepts portable names, and a provider may reject any other
/// with [`ErrorKind::Invalid`]. A portable container name:
///
/// - is 3 to 63 characters long;
/// - holds only lowercase ASCII letters, digits, and `-`;
/// - starts and ends with a letter or digit, and never holds `--`.
///
/// A portable object name is one or more segments joined by `/`. Each segment
/// is not empty, holds only ASCII letters, digits, `.`, `_`, and `-`, and is
/// neither `.` nor `..`. A portable container never holds both an object `a`
/// and an object under `a/`.
#[omnia_guest_macros::capability]
pub trait BlobStore: Send + Sync {
    /// Retrieve the object `name` in `container`, or `None` when it is absent.
    fn get(
        &self, container: &str, name: &str,
    ) -> impl Future<Output = Result<Option<Bytes>>> + Send;

    /// Retrieve the bytes of the object `name` in `range`, or `None` when it is absent.
    ///
    /// The range is half-open and clamped to the object's length, so a range
    /// past the end reads as empty.
    fn get_range(
        &self, container: &str, name: &str, range: Range<u64>,
    ) -> impl Future<Output = Result<Option<Bytes>>> + Send;

    /// Store `data` as the object `name` in `container`, replacing any object there.
    fn put(
        &self, container: &str, name: &str, data: &[u8],
    ) -> impl Future<Output = Result<()>> + Send;

    /// Delete the object `name` from `container`, succeeding when it is already absent.
    fn delete(&self, container: &str, name: &str) -> impl Future<Output = Result<()>> + Send;

    /// The metadata of the object `name` in `container`, or `None` when it is absent.
    fn stat(
        &self, container: &str, name: &str,
    ) -> impl Future<Output = Result<Option<ObjectMetadata>>> + Send;

    /// One page of the objects in `container` whose names start with `prefix`.
    ///
    /// `after` is the [`next`](Page::next) cursor of the previous page, or
    /// `None` for the first page.
    fn list(
        &self, container: &str, prefix: &str, after: Option<&str>,
    ) -> impl Future<Output = Result<Page<ObjectMetadata>>> + Send;

    /// Create the empty container `container`, succeeding when it already exists.
    fn create_container(&self, container: &str) -> impl Future<Output = Result<()>> + Send;

    /// Delete `container` and every object in it, succeeding when it is already absent.
    fn delete_container(&self, container: &str) -> impl Future<Output = Result<()>> + Send;

    /// Whether `container` exists.
    fn container_exists(&self, container: &str) -> impl Future<Output = Result<bool>> + Send;

    /// The metadata of `container`, or `None` when it does not exist.
    fn container_info(
        &self, container: &str,
    ) -> impl Future<Output = Result<Option<ContainerMetadata>>> + Send;

    /// Whether the object `name` exists in `container`.
    fn exists(&self, container: &str, name: &str) -> impl Future<Output = Result<bool>> + Send {
        async move { Ok(self.stat(container, name).await?.is_some()) }
    }

    /// Delete each object in `names` from `container`.
    fn delete_many(
        &self, container: &str, names: &[&str],
    ) -> impl Future<Output = Result<()>> + Send {
        async move {
            for name in names {
                self.delete(container, name).await?;
            }
            Ok(())
        }
    }

    /// Delete every object in `container`, leaving it empty.
    fn clear(&self, container: &str) -> impl Future<Output = Result<()>> + Send {
        async move {
            let mut names = Vec::new();
            let mut after = None;
            loop {
                let page = self.list(container, "", after.as_deref()).await?;
                names.extend(page.items.into_iter().map(|object| object.name));
                match page.next {
                    Some(next) => after = Some(next),
                    None => break,
                }
            }
            for name in names {
                self.delete(container, &name).await?;
            }
            Ok(())
        }
    }

    /// Copy an object to the same or a different container.
    ///
    /// The copy replaces any object at the destination. An absent source fails
    /// with [`ErrorKind::NotFound`].
    fn copy(
        &self, src_container: &str, src_name: &str, dest_container: &str, dest_name: &str,
    ) -> impl Future<Output = Result<()>> + Send {
        async move {
            let data = self.get(src_container, src_name).await?.ok_or_else(|| {
                let message = format!("object `{src_name}` not found in `{src_container}`");
                Error::new(ErrorKind::NotFound, message)
            })?;
            self.put(dest_container, dest_name, &data).await
        }
    }

    /// Move an object to a new name in the same or a different container.
    ///
    /// The move replaces any object at the destination, and fails as
    /// [`copy`](Self::copy) does, leaving the source in place.
    fn rename(
        &self, src_container: &str, src_name: &str, dest_container: &str, dest_name: &str,
    ) -> impl Future<Output = Result<()>> + Send {
        async move {
            if src_container == dest_container && src_name == dest_name {
                return Ok(());
            }
            self.copy(src_container, src_name, dest_container, dest_name).await?;
            self.delete(src_container, src_name).await
        }
    }
}

/// Metadata for a blob store container.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct ContainerMetadata {
    /// The container's name.
    pub name: String,

    /// When the container was created.
    pub created_at: SystemTime,
}

impl ContainerMetadata {
    /// Metadata for the container `name`, created at `created_at`.
    pub fn new(name: impl Into<String>, created_at: SystemTime) -> Self {
        Self {
            name: name.into(),
            created_at,
        }
    }
}

/// Metadata for an object in a blob store container.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct ObjectMetadata {
    /// The object's name.
    pub name: String,

    /// The container holding the object.
    pub container: String,

    /// When the object was written.
    pub created_at: SystemTime,

    /// The object's size in bytes.
    pub size: u64,
}

impl ObjectMetadata {
    /// Metadata for the object `name` in `container`, written at `created_at`.
    pub fn new(
        name: impl Into<String>, container: impl Into<String>, created_at: SystemTime, size: u64,
    ) -> Self {
        Self {
            name: name.into(),
            container: container.into(),
            created_at,
            size,
        }
    }
}
