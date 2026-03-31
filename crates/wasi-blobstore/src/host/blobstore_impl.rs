use wasmtime::component::{Accessor, Resource};

use crate::host::generated::wasi::blobstore::blobstore::{Host, HostWithStore, ObjectId};
use crate::host::resource::ContainerProxy;
use crate::host::{Result, WasiBlobstore, WasiBlobstoreCtxView};

fn same_object(src: &ObjectId, dest: &ObjectId) -> bool {
    src.container == dest.container && src.object == dest.object
}

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

        if same_object(&src, &dest) {
            // No-op for identical source and destination; deleting would corrupt data.
            return Ok(());
        }

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_object_detects_identical_ids() {
        let a = ObjectId {
            container: "bucket".to_string(),
            object: "blob".to_string(),
        };
        let b = ObjectId {
            container: "bucket".to_string(),
            object: "blob".to_string(),
        };
        assert!(same_object(&a, &b));
    }

    #[test]
    fn same_object_rejects_different_ids() {
        let a = ObjectId {
            container: "bucket".to_string(),
            object: "blob".to_string(),
        };
        let b = ObjectId {
            container: "bucket".to_string(),
            object: "blob-2".to_string(),
        };
        assert!(!same_object(&a, &b));
    }
}
