use omnia_core::HostView;
use wasmtime::component::{Accessor, Resource};

use crate::host::container_impl::ContainerHandle;
use crate::host::generated::omnia::blobstore::admin::{Host, HostWithStore, Names};
use crate::host::{BlobStoreClient, Error, Result, WasiBlobStore};

impl<T> HostWithStore<T> for WasiBlobStore {
    async fn create_container(
        accessor: &Accessor<T, Self>, name: String,
    ) -> Result<Resource<ContainerHandle>> {
        let container =
            accessor.with(|mut store| store.get().client.create_container(name.clone())).await?;
        ContainerHandle::push(accessor, name, container)
    }

    async fn delete_container(accessor: &Accessor<T, Self>, name: String) -> Result<()> {
        Ok(accessor.with(|mut store| store.get().client.delete_container(name)).await?)
    }

    fn list_containers(
        _: &Accessor<T, Self>, _: String, _: Option<String>, _: Option<u32>,
    ) -> impl Future<Output = Result<Names>> + Send {
        std::future::ready(Err(Error::Unsupported("this backend lists no containers".to_owned())))
    }
}

impl Host for HostView<'_, dyn BlobStoreClient> {}
