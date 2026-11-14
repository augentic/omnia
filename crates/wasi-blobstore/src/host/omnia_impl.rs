use std::sync::Arc;

use bytes::Bytes;
use omnia_core::HostView;
use wasmtime::component::Accessor;

use crate::host::generated::omnia::blobstore::store::{Host, HostWithStore};
use crate::host::resource::Container;
use crate::host::{
    BlobStoreClient, ContainerMetadata, Error, ObjectMetadata, Page, Result, WasiBlobStore,
};

impl<T> HostWithStore<T> for WasiBlobStore {
    async fn get(
        accessor: &Accessor<T, Self>, container: String, name: String,
    ) -> Result<Option<Vec<u8>>> {
        let data = open(accessor, container).await?.get_data(name, 0..u64::MAX).await?;
        Ok(data.map(Vec::from))
    }

    async fn get_range(
        accessor: &Accessor<T, Self>, container: String, name: String, start: u64, end: u64,
    ) -> Result<Option<Vec<u8>>> {
        let data = open(accessor, container).await?.get_data(name, start..end).await?;
        Ok(data.map(Vec::from))
    }

    async fn put(
        accessor: &Accessor<T, Self>, container: String, name: String, data: Vec<u8>,
    ) -> Result<()> {
        Ok(open(accessor, container).await?.write_data(name, Bytes::from(data)).await?)
    }

    async fn delete(accessor: &Accessor<T, Self>, container: String, name: String) -> Result<()> {
        Ok(open(accessor, container).await?.delete_object(name).await?)
    }

    async fn stat(
        accessor: &Accessor<T, Self>, container: String, name: String,
    ) -> Result<Option<ObjectMetadata>> {
        Ok(open(accessor, container).await?.object_info(name).await?)
    }

    async fn list(
        accessor: &Accessor<T, Self>, container: String, prefix: String, after: Option<String>,
    ) -> Result<Page> {
        Ok(open(accessor, container).await?.list_objects(prefix, after).await?)
    }

    async fn create_container(accessor: &Accessor<T, Self>, container: String) -> Result<()> {
        accessor.with(|mut store| store.get().client.create_container(container)).await?;
        Ok(())
    }

    async fn delete_container(accessor: &Accessor<T, Self>, container: String) -> Result<()> {
        Ok(accessor.with(|mut store| store.get().client.delete_container(container)).await?)
    }

    async fn container_exists(accessor: &Accessor<T, Self>, container: String) -> Result<bool> {
        Ok(accessor.with(|mut store| store.get().client.container_exists(container)).await?)
    }

    async fn container_info(
        accessor: &Accessor<T, Self>, container: String,
    ) -> Result<Option<ContainerMetadata>> {
        let exists = accessor
            .with(|mut store| store.get().client.container_exists(container.clone()))
            .await?;
        if !exists {
            return Ok(None);
        }
        Ok(Some(open(accessor, container).await?.info()?))
    }

    async fn exists(accessor: &Accessor<T, Self>, container: String, name: String) -> Result<bool> {
        Ok(open(accessor, container).await?.has_object(name).await?)
    }

    async fn delete_many(
        accessor: &Accessor<T, Self>, container: String, names: Vec<String>,
    ) -> Result<()> {
        let container = open(accessor, container).await?;
        for name in names {
            container.delete_object(name).await?;
        }
        Ok(())
    }

    async fn clear(accessor: &Accessor<T, Self>, container: String) -> Result<()> {
        let container = open(accessor, container).await?;
        for name in container.names().await? {
            container.delete_object(name).await?;
        }
        Ok(())
    }

    async fn copy(
        accessor: &Accessor<T, Self>, src_container: String, src_name: String,
        dest_container: String, dest_name: String,
    ) -> Result<()> {
        let source = open(accessor, src_container.clone()).await?;
        let Some(data) = source.get_data(src_name.clone(), 0..u64::MAX).await? else {
            let message = format!("source object not found: {src_container}/{src_name}");
            return Err(Error::NotFound(message));
        };
        Ok(open(accessor, dest_container).await?.write_data(dest_name, data).await?)
    }

    async fn rename(
        accessor: &Accessor<T, Self>, src_container: String, src_name: String,
        dest_container: String, dest_name: String,
    ) -> Result<()> {
        // deleting the source would destroy the destination
        if src_container == dest_container && src_name == dest_name {
            return Ok(());
        }
        Self::copy(accessor, src_container.clone(), src_name.clone(), dest_container, dest_name)
            .await?;
        Ok(open(accessor, src_container).await?.delete_object(src_name).await?)
    }
}

async fn open<T>(
    accessor: &Accessor<T, WasiBlobStore>, container: String,
) -> anyhow::Result<Arc<dyn Container>> {
    accessor.with(|mut store| store.get().client.get_container(container)).await
}

impl Host for HostView<'_, dyn BlobStoreClient> {
    fn convert_error(&mut self, err: Error) -> wasmtime::Result<Error> {
        Ok(err)
    }
}
