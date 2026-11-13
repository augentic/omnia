//! Binary large object storage capability.

use std::future::Future;
use std::ops::Range;

use bytes::Bytes;

use crate::blobstore::{ContainerMetadata, ObjectMetadata};
use crate::capability::{Error, ErrorKind, Page, Result};

/// Store and retrieve binary objects in a deployment's containers.
///
/// An absent object is `Ok(None)`, and a call on an object in a container
/// that does not exist fails with [`ErrorKind::NotFound`].
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

#[cfg(all(target_arch = "wasm32", target_os = "wasi"))]
mod wasi {
    use std::ops::Range;
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    use bytes::Bytes;
    use omnia_wasi_blobstore::blobstore;
    use omnia_wasi_blobstore::container::Container;
    use omnia_wasi_blobstore::types::{IncomingValue, ObjectId, OutgoingValue};

    use super::BlobStore;
    use crate::blobstore::{ContainerMetadata, ObjectMetadata};
    use crate::capability::{Error, ErrorKind, Page, Result};

    // the `wasi:io` write budget: `blocking-write-and-flush` takes at most 4096 bytes
    const WRITE_CHUNK: usize = 4096;

    // `wasi-blobstore` lowers a missing object to this message for every backend
    const OBJECT_NOT_FOUND: &str = "object not found";

    impl BlobStore for crate::WasiProvider {
        async fn get(&self, container: &str, name: &str) -> Result<Option<Bytes>> {
            let container = open(container).await?;
            match container.get_data(name.to_owned(), 0, u64::MAX).await {
                Ok(incoming) => consume(incoming).map(Some),
                Err(message) if message.contains(OBJECT_NOT_FOUND) => Ok(None),
                Err(message) => Err(fault("reading object", &message)),
            }
        }

        async fn get_range(
            &self, container: &str, name: &str, range: Range<u64>,
        ) -> Result<Option<Bytes>> {
            let container = open(container).await?;
            if range.is_empty() {
                let found = container
                    .has_object(name.to_owned())
                    .await
                    .map_err(|message| fault("checking object", &message))?;
                return Ok(found.then(Bytes::new));
            }

            // HACK: the wire end is inclusive and reads an end of 0 as unbounded,
            // so a read of the first byte asks for two and truncates
            let end = (range.end - 1).max(1);
            let data = match container.get_data(name.to_owned(), range.start, end).await {
                Ok(incoming) => consume(incoming)?,
                Err(message) if message.contains(OBJECT_NOT_FOUND) => return Ok(None),
                Err(message) => return Err(fault("reading object range", &message)),
            };
            let len = usize::try_from(range.end - range.start).unwrap_or(usize::MAX);
            Ok(Some(data.slice(..len.min(data.len()))))
        }

        async fn put(&self, container: &str, name: &str, data: &[u8]) -> Result<()> {
            let container = open(container).await?;
            let outgoing = OutgoingValue::new_outgoing_value();
            {
                let body = outgoing.outgoing_value_write_body().await.map_err(|err| {
                    Error::new(ErrorKind::Other, format!("opening the write body: {err:?}"))
                })?;
                for chunk in data.chunks(WRITE_CHUNK) {
                    body.blocking_write_and_flush(chunk).map_err(|err| {
                        Error::new(ErrorKind::Other, format!("writing object data: {err}"))
                    })?;
                }
            };
            container
                .write_data(name.to_owned(), &outgoing)
                .await
                .map_err(|message| fault("writing object", &message))?;
            OutgoingValue::finish(outgoing).map_err(|message| fault("finishing write", &message))
        }

        async fn delete(&self, container: &str, name: &str) -> Result<()> {
            let container = open(container).await?;
            container
                .delete_object(name.to_owned())
                .await
                .map_err(|message| fault("deleting object", &message))
        }

        async fn stat(&self, container: &str, name: &str) -> Result<Option<ObjectMetadata>> {
            let container = open(container).await?;
            stat_in(&container, name).await
        }

        async fn list(
            &self, container: &str, prefix: &str, after: Option<&str>,
        ) -> Result<Page<ObjectMetadata>> {
            let container = open(container).await?;
            let stream = container
                .list_objects()
                .await
                .map_err(|message| fault("listing objects", &message))?;
            let mut names = Vec::new();
            loop {
                let (batch, done) = stream
                    .read_stream_object_names(100)
                    .await
                    .map_err(|message| fault("reading object names", &message))?;
                names.extend(batch);
                if done {
                    break;
                }
            }

            // `wasi:blobstore` lists every name, so the listing filters and orders
            // them into one page that resumes after the name it is given
            names.retain(|name| {
                name.starts_with(prefix) && after.is_none_or(|after| name.as_str() > after)
            });
            names.sort_unstable();
            let mut objects = Vec::with_capacity(names.len());
            for name in names {
                objects.extend(stat_in(&container, &name).await?);
            }
            Ok(Page::new(objects, None))
        }

        async fn create_container(&self, container: &str) -> Result<()> {
            blobstore::create_container(container.to_owned())
                .await
                .map(drop)
                .map_err(|message| fault("creating container", &message))
        }

        async fn delete_container(&self, container: &str) -> Result<()> {
            blobstore::delete_container(container.to_owned())
                .await
                .map_err(|message| fault("deleting container", &message))
        }

        async fn container_exists(&self, container: &str) -> Result<bool> {
            blobstore::container_exists(container.to_owned())
                .await
                .map_err(|message| fault("checking container", &message))
        }

        async fn container_info(&self, container: &str) -> Result<Option<ContainerMetadata>> {
            if !self.container_exists(container).await? {
                return Ok(None);
            }
            let info = open(container)
                .await?
                .info()
                .map_err(|message| fault("reading container info", &message))?;
            Ok(Some(ContainerMetadata::new(info.name, timestamp(info.created_at))))
        }

        async fn exists(&self, container: &str, name: &str) -> Result<bool> {
            let container = open(container).await?;
            container
                .has_object(name.to_owned())
                .await
                .map_err(|message| fault("checking object", &message))
        }

        async fn delete_many(&self, container: &str, names: &[&str]) -> Result<()> {
            let container = open(container).await?;
            let names = names.iter().map(|&name| name.to_owned()).collect();
            container
                .delete_objects(names)
                .await
                .map_err(|message| fault("deleting objects", &message))
        }

        async fn clear(&self, container: &str) -> Result<()> {
            let container = open(container).await?;
            container.clear().await.map_err(|message| fault("clearing container", &message))
        }

        async fn copy(
            &self, src_container: &str, src_name: &str, dest_container: &str, dest_name: &str,
        ) -> Result<()> {
            blobstore::copy_object(
                object_id(src_container, src_name),
                object_id(dest_container, dest_name),
            )
            .await
            .map_err(|message| fault("copying object", &message))
        }

        async fn rename(
            &self, src_container: &str, src_name: &str, dest_container: &str, dest_name: &str,
        ) -> Result<()> {
            blobstore::move_object(
                object_id(src_container, src_name),
                object_id(dest_container, dest_name),
            )
            .await
            .map_err(|message| fault("moving object", &message))
        }
    }

    async fn open(container: &str) -> Result<Container> {
        blobstore::get_container(container.to_owned())
            .await
            .map_err(|message| fault("opening container", &message))
    }

    // a missing object fails `object-info` with a backend's own message, so the
    // failure is read as absence only once `has-object` agrees
    async fn stat_in(container: &Container, name: &str) -> Result<Option<ObjectMetadata>> {
        match container.object_info(name.to_owned()).await {
            Ok(info) => Ok(Some(ObjectMetadata::new(
                info.name,
                info.container,
                timestamp(info.created_at),
                info.size,
            ))),
            Err(message) => {
                let found = container
                    .has_object(name.to_owned())
                    .await
                    .map_err(|message| fault("checking object", &message))?;
                if found { Err(fault("reading object info", &message)) } else { Ok(None) }
            }
        }
    }

    fn consume(incoming: IncomingValue) -> Result<Bytes> {
        IncomingValue::incoming_value_consume_sync(incoming)
            .map(Bytes::from)
            .map_err(|message| fault("reading object data", &message))
    }

    fn object_id(container: &str, name: &str) -> ObjectId {
        ObjectId {
            container: container.to_owned(),
            object: name.to_owned(),
        }
    }

    fn timestamp(seconds: u64) -> SystemTime {
        UNIX_EPOCH + Duration::from_secs(seconds)
    }

    // `wasi:blobstore` errors are strings, so a missing container or object is
    // told apart by its message
    fn fault(action: &str, message: &str) -> Error {
        let kind =
            if message.contains("not found") { ErrorKind::NotFound } else { ErrorKind::Other };
        Error::new(kind, format!("{action}: {message}"))
    }
}
