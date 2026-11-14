//! [`BlobStore`] over `omnia:blobstore`.

use std::ops::Range;
use std::time::{Duration, UNIX_EPOCH};

use bytes::Bytes;
use omnia_wasi_blobstore::omnia::store;

use crate::blobstore::{ContainerMetadata, ObjectMetadata};
use crate::{BlobStore, Error, Page, Result, WasiProvider};

impl BlobStore for WasiProvider {
    async fn get(&self, container: &str, name: &str) -> Result<Option<Bytes>> {
        let data = store::get(container.to_owned(), name.to_owned()).await?;
        Ok(data.map(Bytes::from))
    }

    async fn get_range(
        &self, container: &str, name: &str, range: Range<u64>,
    ) -> Result<Option<Bytes>> {
        let data =
            store::get_range(container.to_owned(), name.to_owned(), range.start, range.end).await?;
        Ok(data.map(Bytes::from))
    }

    async fn put(&self, container: &str, name: &str, data: &[u8]) -> Result<()> {
        Ok(store::put(container.to_owned(), name.to_owned(), data.to_vec()).await?)
    }

    async fn delete(&self, container: &str, name: &str) -> Result<()> {
        Ok(store::delete(container.to_owned(), name.to_owned()).await?)
    }

    async fn stat(&self, container: &str, name: &str) -> Result<Option<ObjectMetadata>> {
        let info = store::stat(container.to_owned(), name.to_owned()).await?;
        Ok(info.map(Into::into))
    }

    async fn list(
        &self, container: &str, prefix: &str, after: Option<&str>,
    ) -> Result<Page<ObjectMetadata>> {
        let after = after.map(str::to_owned);
        let page = store::list(container.to_owned(), prefix.to_owned(), after).await?;
        Ok(page.into())
    }

    async fn create_container(&self, container: &str) -> Result<()> {
        Ok(store::create_container(container.to_owned()).await?)
    }

    async fn delete_container(&self, container: &str) -> Result<()> {
        Ok(store::delete_container(container.to_owned()).await?)
    }

    async fn container_exists(&self, container: &str) -> Result<bool> {
        Ok(store::container_exists(container.to_owned()).await?)
    }

    async fn container_info(&self, container: &str) -> Result<Option<ContainerMetadata>> {
        let info = store::container_info(container.to_owned()).await?;
        Ok(info.map(Into::into))
    }

    async fn exists(&self, container: &str, name: &str) -> Result<bool> {
        Ok(store::exists(container.to_owned(), name.to_owned()).await?)
    }

    async fn delete_many(&self, container: &str, names: &[&str]) -> Result<()> {
        let names = names.iter().map(|&name| name.to_owned()).collect();
        Ok(store::delete_many(container.to_owned(), names).await?)
    }

    async fn clear(&self, container: &str) -> Result<()> {
        Ok(store::clear(container.to_owned()).await?)
    }

    async fn copy(
        &self, src_container: &str, src_name: &str, dest_container: &str, dest_name: &str,
    ) -> Result<()> {
        Ok(store::copy(
            src_container.to_owned(),
            src_name.to_owned(),
            dest_container.to_owned(),
            dest_name.to_owned(),
        )
        .await?)
    }

    async fn rename(
        &self, src_container: &str, src_name: &str, dest_container: &str, dest_name: &str,
    ) -> Result<()> {
        Ok(store::rename(
            src_container.to_owned(),
            src_name.to_owned(),
            dest_container.to_owned(),
            dest_name.to_owned(),
        )
        .await?)
    }
}

impl From<store::Error> for Error {
    fn from(err: store::Error) -> Self {
        match err {
            store::Error::NotFound(message) => Self::server_error("not-found", message),
            store::Error::Denied(message) => Self::server_error("denied", message),
            store::Error::Invalid(message) => Self::server_error("invalid", message),
            store::Error::Unsupported(message) => Self::server_error("unsupported", message),
            store::Error::Unavailable(message) => Self::bad_gateway("unavailable", message),
            store::Error::Other(message) => Self::server_error("other", message),
        }
    }
}

impl From<store::ObjectMetadata> for ObjectMetadata {
    fn from(info: store::ObjectMetadata) -> Self {
        let created_at = UNIX_EPOCH + Duration::from_secs(info.created_at);
        Self::new(info.name, info.container, created_at, info.size)
    }
}

impl From<store::ContainerMetadata> for ContainerMetadata {
    fn from(info: store::ContainerMetadata) -> Self {
        Self::new(info.name, UNIX_EPOCH + Duration::from_secs(info.created_at))
    }
}

impl From<store::Page> for Page<ObjectMetadata> {
    fn from(page: store::Page) -> Self {
        Self::new(page.objects.into_iter().map(Into::into).collect(), page.next)
    }
}
