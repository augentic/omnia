use anyhow::Context;
use bytes::Bytes;
use wasmtime::component::{Access, Accessor, Resource};

use crate::host::generated::wasi::blobstore::container::{
    ContainerMetadata, Host, HostContainer, HostContainerWithStore, HostStreamObjectNames,
    HostStreamObjectNamesWithStore, ObjectMetadata,
};
use crate::host::resource::ContainerProxy;
use crate::host::{
    Error, IncomingValue, OutgoingValue, Result, StreamObjectNames, WasiBlobstore,
    WasiBlobstoreCtxView,
};

impl<T> HostContainerWithStore<T> for WasiBlobstore {
    fn name(mut host: Access<'_, T, Self>, self_: Resource<ContainerProxy>) -> Result<String> {
        let container = host.get().table.get(&self_).context("Container not found")?;
        Ok(container.name().context("getting name")?)
    }

    fn info(
        mut host: Access<'_, T, Self>, self_: Resource<ContainerProxy>,
    ) -> Result<ContainerMetadata> {
        let container = host.get().table.get(&self_).context("Container not found")?;
        Ok(container.info().context("getting info")?)
    }

    async fn get_data(
        accessor: &Accessor<T, Self>, self_: Resource<ContainerProxy>, name: String, start: u64,
        end: u64,
    ) -> Result<Resource<IncomingValue>> {
        let container = get_container(accessor, &self_)?;

        let data_opt =
            container.get_data(name.clone(), start, end).await.context("getting data")?;

        let Some(data) = data_opt else {
            return Err(Error::NotFound(format!("object not found: {name}")));
        };

        Ok(accessor.with(|mut store| store.get().table.push(data))?)
    }

    async fn write_data(
        accessor: &Accessor<T, Self>, self_: Resource<ContainerProxy>, name: String,
        data: Resource<OutgoingValue>,
    ) -> Result<()> {
        let bytes = accessor.with(|mut store| {
            let value = store.get().table.get(&data)?;
            Ok::<Bytes, Error>(value.pipe.contents())
        })?;

        let container = get_container(accessor, &self_)?;
        container.write_data(name, bytes).await.context("writing data")?;

        Ok(())
    }

    async fn list_objects(
        accessor: &Accessor<T, Self>, self_: Resource<ContainerProxy>,
    ) -> Result<Resource<StreamObjectNames>> {
        let container = get_container(accessor, &self_)?;
        let names = container.list_objects().await.context("listing objects")?;
        let stream = StreamObjectNames::new(names);
        Ok(accessor.with(|mut store| store.get().table.push(stream))?)
    }

    async fn delete_object(
        accessor: &Accessor<T, Self>, self_: Resource<ContainerProxy>, name: String,
    ) -> Result<()> {
        let container = get_container(accessor, &self_)?;
        container.delete_object(name).await.context("deleting object")?;
        Ok(())
    }

    async fn delete_objects(
        accessor: &Accessor<T, Self>, self_: Resource<ContainerProxy>, names: Vec<String>,
    ) -> Result<()> {
        let container = get_container(accessor, &self_)?;
        for name in names {
            container.delete_object(name).await.context("deleting object")?;
        }

        Ok(())
    }

    async fn has_object(
        accessor: &Accessor<T, Self>, self_: Resource<ContainerProxy>, name: String,
    ) -> Result<bool> {
        let container = get_container(accessor, &self_)?;
        Ok(container.has_object(name).await.context("checking object exists")?)
    }

    async fn object_info(
        accessor: &Accessor<T, Self>, self_: Resource<ContainerProxy>, name: String,
    ) -> Result<ObjectMetadata> {
        let container = get_container(accessor, &self_)?;
        Ok(container.object_info(name).await.context("getting object info")?)
    }

    async fn clear(accessor: &Accessor<T, Self>, self_: Resource<ContainerProxy>) -> Result<()> {
        let container = get_container(accessor, &self_)?;

        let all_objects = container.list_objects().await.context("listing objects")?;

        for name in all_objects {
            container.delete_object(name).await.context("deleting object")?;
        }

        Ok(())
    }

    fn drop(
        mut accessor: Access<'_, T, Self>, rep: Resource<ContainerProxy>,
    ) -> wasmtime::Result<()> {
        Ok(accessor.get().table.delete(rep).map(|_| ())?)
    }
}

impl<T> HostStreamObjectNamesWithStore<T> for WasiBlobstore {
    async fn read_stream_object_names(
        accessor: &Accessor<T, Self>, self_: Resource<StreamObjectNames>, len: u64,
    ) -> Result<(Vec<String>, bool)> {
        accessor.with(|mut store| {
            let stream =
                store.get().table.get_mut(&self_).context("StreamObjectNames not found")?;

            let remaining = &stream.names[stream.offset..];
            let take = usize::try_from(len).unwrap_or(usize::MAX).min(remaining.len());
            let batch = remaining[..take].to_vec();
            stream.offset += take;
            let done = stream.offset >= stream.names.len();
            Ok((batch, done))
        })
    }

    async fn skip_stream_object_names(
        accessor: &Accessor<T, Self>, self_: Resource<StreamObjectNames>, num: u64,
    ) -> Result<(u64, bool)> {
        accessor.with(|mut store| {
            let stream =
                store.get().table.get_mut(&self_).context("StreamObjectNames not found")?;

            let remaining = stream.names.len() - stream.offset;
            let skip = usize::try_from(num).unwrap_or(usize::MAX).min(remaining);
            stream.offset += skip;
            let done = stream.offset >= stream.names.len();
            Ok((skip as u64, done))
        })
    }

    fn drop(
        mut accessor: Access<'_, T, Self>, rep: Resource<StreamObjectNames>,
    ) -> wasmtime::Result<()> {
        Ok(accessor.get().table.delete(rep).map(|_| ())?)
    }
}

impl Host for WasiBlobstoreCtxView<'_> {}
impl HostContainer for WasiBlobstoreCtxView<'_> {}
impl HostStreamObjectNames for WasiBlobstoreCtxView<'_> {}

pub fn get_container<T>(
    accessor: &Accessor<T, WasiBlobstore>, self_: &Resource<ContainerProxy>,
) -> Result<ContainerProxy> {
    accessor.with(|mut store| {
        let container = store.get().table.get(self_).context("Container not found")?;
        Ok(container.clone())
    })
}
