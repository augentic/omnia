use omnia_core::HostView;
use wasmtime::component::{Accessor, Resource};

use crate::container_impl::ContainerHandle;
use crate::generated::omnia::blobstore::store::{Host, HostWithStore};
use crate::{BlobStoreClient, Result, WasiBlobStore};

impl<T> HostWithStore<T> for WasiBlobStore {
    async fn open(accessor: &Accessor<T, Self>, name: String) -> Result<Resource<ContainerHandle>> {
        let container =
            accessor.with(|mut store| store.get().client.get_container(name.clone())).await?;
        ContainerHandle::push(accessor, name, container)
    }

    async fn container_exists(accessor: &Accessor<T, Self>, name: String) -> Result<bool> {
        Ok(accessor.with(|mut store| store.get().client.container_exists(name)).await?)
    }
}

impl Host for HostView<'_, dyn BlobStoreClient> {}
