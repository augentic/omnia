//! [`BlobStore`] over `omnia:blobstore`.

use std::ops::Range;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use bytes::Bytes;
use omnia_wasi_blobstore::omnia::container::{ByteRange, Content, Listing};
use omnia_wasi_blobstore::omnia::types::{Attributes, Conflict};
use omnia_wasi_blobstore::omnia::{admin, container, store, types};
use omnia_wasi_blobstore::{wit_future, wit_stream};

use crate::blobstore::{ContainerMetadata, ObjectMetadata};
use crate::{BlobStore, Error, Page, Result, WasiProvider};

impl BlobStore for WasiProvider {
    async fn get(&self, container: &str, name: &str) -> Result<Option<Bytes>> {
        let handle = store::open(container.to_owned()).await?;
        read(handle.get(name.to_owned(), None).await?).await
    }

    async fn get_range(
        &self, container: &str, name: &str, range: Range<u64>,
    ) -> Result<Option<Bytes>> {
        let handle = store::open(container.to_owned()).await?;
        let range = ByteRange::Span((range.start, range.end));
        read(handle.get(name.to_owned(), Some(range)).await?).await
    }

    async fn put(&self, container: &str, name: &str, data: &[u8]) -> Result<()> {
        let handle = store::open(container.to_owned()).await?;
        let (mut writer, reader) = wit_stream::new();
        let (finish, finished) =
            wit_future::new(|| Err(types::Error::Other("the write was abandoned".to_owned())));
        let attributes = Attributes {
            content_type: None,
            content_encoding: None,
            content_language: None,
            content_disposition: None,
            cache_control: None,
            metadata: Vec::new(),
        };
        let reply = handle.put(name, reader, finished, &attributes, None);

        // a short write or a refused `finished` means the host refused the
        // write, and its reply says why
        writer.write_all(data.to_vec()).await;
        drop(writer);
        let _ = finish.write(Ok(())).await;
        reply.await??;
        Ok(())
    }

    async fn delete(&self, container: &str, name: &str) -> Result<()> {
        let handle = store::open(container.to_owned()).await?;
        handle.delete(name.to_owned(), None).await??;
        Ok(())
    }

    async fn stat(&self, container: &str, name: &str) -> Result<Option<ObjectMetadata>> {
        let handle = store::open(container.to_owned()).await?;
        let info = handle.stat(name.to_owned()).await?;
        Ok(info.map(|info| {
            ObjectMetadata::new(info.name, container, since_epoch(info.modified_at), info.size)
        }))
    }

    async fn list(
        &self, container: &str, prefix: &str, after: Option<&str>,
    ) -> Result<Page<ObjectMetadata>> {
        let handle = store::open(container.to_owned()).await?;
        let after = after.map(str::to_owned);
        let page = handle.list(prefix.to_owned(), Listing::Flat, after, None).await?;
        let objects = page.objects.into_iter().map(|object| {
            ObjectMetadata::new(
                object.name,
                container,
                since_epoch(object.modified_at),
                object.size,
            )
        });
        Ok(Page::new(objects.collect(), page.next))
    }

    async fn create_container(&self, container: &str) -> Result<()> {
        admin::create_container(container.to_owned()).await?;
        Ok(())
    }

    async fn delete_container(&self, container: &str) -> Result<()> {
        Ok(admin::delete_container(container.to_owned()).await?)
    }

    async fn container_exists(&self, container: &str) -> Result<bool> {
        Ok(store::container_exists(container.to_owned()).await?)
    }

    async fn container_info(&self, container: &str) -> Result<Option<ContainerMetadata>> {
        let info = match store::open(container.to_owned()).await {
            Ok(handle) => handle.info().await,
            Err(error) => Err(error),
        };
        match info {
            Ok(info) => Ok(Some(info.into())),
            Err(types::Error::NotFound(_)) => Ok(None),
            Err(error) => Err(error.into()),
        }
    }

    async fn exists(&self, container: &str, name: &str) -> Result<bool> {
        let handle = store::open(container.to_owned()).await?;
        Ok(handle.exists(name.to_owned()).await?)
    }

    async fn delete_many(&self, container: &str, names: &[&str]) -> Result<()> {
        let handle = store::open(container.to_owned()).await?;
        let names = names.iter().map(|&name| name.to_owned()).collect();
        Ok(handle.delete_many(names).await?)
    }

    async fn clear(&self, container: &str) -> Result<()> {
        let handle = store::open(container.to_owned()).await?;
        Ok(handle.clear().await?)
    }

    async fn copy(
        &self, src_container: &str, src_name: &str, dest_container: &str, dest_name: &str,
    ) -> Result<()> {
        let source = store::open(src_container.to_owned()).await?;
        let dest = store::open(dest_container.to_owned()).await?;
        source.copy(src_name.to_owned(), &dest, dest_name.to_owned(), None).await??;
        Ok(())
    }

    async fn rename(
        &self, src_container: &str, src_name: &str, dest_container: &str, dest_name: &str,
    ) -> Result<()> {
        let source = store::open(src_container.to_owned()).await?;
        let dest = store::open(dest_container.to_owned()).await?;
        source.rename(src_name.to_owned(), &dest, dest_name.to_owned(), None).await??;
        Ok(())
    }
}

// the bytes `content` carries, once its `finished` future vouches for them
async fn read(content: Option<Content>) -> Result<Option<Bytes>> {
    let Some(Content { data, finished, .. }) = content else {
        return Ok(None);
    };
    let bytes = data.collect().await;
    finished.await?;
    Ok(Some(Bytes::from(bytes)))
}

fn since_epoch(seconds: u64) -> SystemTime {
    UNIX_EPOCH + Duration::from_secs(seconds)
}

impl From<types::Error> for Error {
    fn from(err: types::Error) -> Self {
        match err {
            types::Error::NotFound(message) => Self::server_error("not-found", message),
            types::Error::Denied(message) => Self::server_error("denied", message),
            types::Error::Invalid(message) => Self::server_error("invalid", message),
            types::Error::Unsupported(message) => Self::server_error("unsupported", message),
            types::Error::Unavailable(message) => Self::bad_gateway("unavailable", message),
            types::Error::Other(message) => Self::server_error("other", message),
        }
    }
}

impl From<Conflict> for Error {
    fn from(conflict: Conflict) -> Self {
        Self::server_error("conflict", format!("`{}` failed its precondition", conflict.name))
    }
}

impl From<container::ContainerMetadata> for ContainerMetadata {
    fn from(info: container::ContainerMetadata) -> Self {
        Self::new(info.name, since_epoch(info.created_at.unwrap_or_default()))
    }
}
