use wasmtime::component::{Accessor, Resource};

use crate::host::generated::wasi::blobstore::blobstore::{Host, HostWithStore, ObjectId};
use crate::host::resource::ContainerProxy;
use crate::host::{Result, WasiBlobstore, WasiBlobstoreCtxView};

impl HostWithStore for WasiBlobstore {
    async fn create_container<T>(
        accessor: &Accessor<T, Self>, name: String,
    ) -> Result<Resource<ContainerProxy>> {
        tracing::trace!("create_container: {name}");
        let container = accessor
            .with(|mut store| store.get().ctx.create_container(name))
            .await
            .map_err(|e| e.to_string())?;
        let proxy = ContainerProxy(container);
        accessor.with(|mut store| store.get().table.push(proxy)).map_err(|e| e.to_string())
    }

    async fn get_container<T>(
        accessor: &Accessor<T, Self>, name: String,
    ) -> Result<Resource<ContainerProxy>> {
        tracing::trace!("get_container: {name}");
        let container = accessor
            .with(|mut store| store.get().ctx.get_container(name))
            .await
            .map_err(|e| e.to_string())?;
        let proxy = ContainerProxy(container);
        accessor.with(|mut store| store.get().table.push(proxy)).map_err(|e| e.to_string())
    }

    async fn delete_container<T>(accessor: &Accessor<T, Self>, name: String) -> Result<()> {
        tracing::trace!("delete_container: {name}");
        accessor
            .with(|mut store| store.get().ctx.delete_container(name))
            .await
            .map_err(|e| e.to_string())
    }

    async fn container_exists<T>(accessor: &Accessor<T, Self>, name: String) -> Result<bool> {
        tracing::trace!("container_exists: {name}");
        accessor
            .with(|mut store| store.get().ctx.container_exists(name))
            .await
            .map_err(|e| e.to_string())
    }

    async fn copy_object<T>(
        accessor: &Accessor<T, Self>, src: ObjectId, dest: ObjectId,
    ) -> Result<()> {
        tracing::trace!(
            "copy_object: {}/{} -> {}/{}",
            src.container,
            src.object,
            dest.container,
            dest.object
        );

        let src_container = accessor
            .with(|mut store| store.get().ctx.get_container(src.container.clone()))
            .await
            .map_err(|e| e.to_string())?;

        let data = src_container
            .get_data(src.object.clone(), 0, u64::MAX)
            .await
            .map_err(|e| e.to_string())?
            .ok_or_else(|| format!("source object not found: {}/{}", src.container, src.object))?;

        let dest_container = accessor
            .with(|mut store| store.get().ctx.get_container(dest.container.clone()))
            .await
            .map_err(|e| e.to_string())?;

        dest_container.write_data(dest.object, data).await.map_err(|e| e.to_string())
    }

    async fn move_object<T>(
        accessor: &Accessor<T, Self>, src: ObjectId, dest: ObjectId,
    ) -> Result<()> {
        tracing::trace!(
            "move_object: {}/{} -> {}/{}",
            src.container,
            src.object,
            dest.container,
            dest.object
        );

        let src_container = accessor
            .with(|mut store| store.get().ctx.get_container(src.container.clone()))
            .await
            .map_err(|e| e.to_string())?;

        let src_object_name = src.object.clone();
        let data = src_container
            .get_data(src.object.clone(), 0, u64::MAX)
            .await
            .map_err(|e| e.to_string())?
            .ok_or_else(|| format!("source object not found: {}/{}", src.container, src.object))?;

        let dest_container = accessor
            .with(|mut store| store.get().ctx.get_container(dest.container.clone()))
            .await
            .map_err(|e| e.to_string())?;

        dest_container.write_data(dest.object, data).await.map_err(|e| e.to_string())?;

        src_container.delete_object(src_object_name).await.map_err(|e| e.to_string())
    }
}

impl Host for WasiBlobstoreCtxView<'_> {}
