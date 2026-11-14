use anyhow::Context;
use omnia_core::HostView;
use wasmtime::component::{Accessor, Resource};

use crate::host::generated::omnia::blobstore::store;
use crate::host::generated::wasi::blobstore::blobstore::{Host, HostWithStore, ObjectId};
use crate::host::resource::ContainerProxy;
use crate::host::{BlobStoreClient, Result, WasiBlobStore};

impl<T> HostWithStore<T> for WasiBlobStore {
    async fn create_container(
        accessor: &Accessor<T, Self>, name: String,
    ) -> Result<Resource<ContainerProxy>> {
        tracing::trace!("create_container: {name}");
        let container = accessor
            .with(|mut store| store.get().client.create_container(name))
            .await
            .context("creating container")?;
        let proxy = omnia_core::Proxy(container);
        Ok(accessor.with(|mut store| store.get().table.push(proxy))?)
    }

    async fn get_container(
        accessor: &Accessor<T, Self>, name: String,
    ) -> Result<Resource<ContainerProxy>> {
        tracing::trace!("get_container: {name}");
        let container = accessor
            .with(|mut store| store.get().client.get_container(name))
            .await
            .context("getting container")?;
        let proxy = omnia_core::Proxy(container);
        Ok(accessor.with(|mut store| store.get().table.push(proxy))?)
    }

    async fn delete_container(accessor: &Accessor<T, Self>, name: String) -> Result<()> {
        tracing::trace!("delete_container: {name}");
        accessor
            .with(|mut store| store.get().client.delete_container(name))
            .await
            .context("deleting container")?;
        Ok(())
    }

    async fn container_exists(accessor: &Accessor<T, Self>, name: String) -> Result<bool> {
        tracing::trace!("container_exists: {name}");
        Ok(accessor
            .with(|mut store| store.get().client.container_exists(name))
            .await
            .context("checking container exists")?)
    }

    async fn copy_object(
        accessor: &Accessor<T, Self>, src: ObjectId, dest: ObjectId,
    ) -> Result<()> {
        tracing::trace!(
            "copy_object: {}/{} -> {}/{}",
            src.container,
            src.object,
            dest.container,
            dest.object
        );
        <Self as store::HostWithStore<T>>::copy(
            accessor,
            src.container,
            src.object,
            dest.container,
            dest.object,
        )
        .await
    }

    async fn move_object(
        accessor: &Accessor<T, Self>, src: ObjectId, dest: ObjectId,
    ) -> Result<()> {
        tracing::trace!(
            "move_object: {}/{} -> {}/{}",
            src.container,
            src.object,
            dest.container,
            dest.object
        );
        <Self as store::HostWithStore<T>>::rename(
            accessor,
            src.container,
            src.object,
            dest.container,
            dest.object,
        )
        .await
    }
}

impl Host for HostView<'_, dyn BlobStoreClient> {}
