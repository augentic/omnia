use anyhow::Context;
use bytes::{Bytes, BytesMut};
use wasmtime::component::{Access, Accessor, Resource};

use crate::host::generated::wasi::blobstore::container::{
    ContainerMetadata, Host, HostContainer, HostContainerWithStore, HostStreamObjectNames,
    HostStreamObjectNamesWithStore, ObjectMetadata,
};
use crate::host::resource::ContainerProxy;
use crate::host::{OutgoingValue, Result, StreamObjectNames, WasiBlobstore, WasiBlobstoreCtxView};

pub type IncomingValue = Bytes;

impl HostContainerWithStore for WasiBlobstore {
    fn name<T>(mut host: Access<'_, T, Self>, self_: Resource<ContainerProxy>) -> Result<String> {
        let container = host
            .get()
            .table
            .get(&self_)
            .context("Container not found")
            .map_err(|e| e.to_string())?;

        container.name().context("getting name").map_err(|e| e.to_string())
    }

    fn info<T>(
        mut host: Access<'_, T, Self>, self_: Resource<ContainerProxy>,
    ) -> Result<ContainerMetadata> {
        let container = host
            .get()
            .table
            .get(&self_)
            .context("Container not found")
            .map_err(|e| e.to_string())?;

        container.info().context("getting info").map_err(|e| e.to_string())
    }

    async fn get_data<T>(
        accessor: &Accessor<T, Self>, self_: Resource<ContainerProxy>, name: String, start: u64,
        end: u64,
    ) -> Result<Resource<IncomingValue>> {
        let container = get_container(accessor, &self_)?;

        let data_opt = container
            .get_data(name, start, end)
            .await
            .context("getting data")
            .map_err(|e| e.to_string())?;

        let Some(data) = data_opt else {
            return Err("object not found".to_string());
        };
        let buf = BytesMut::from(&*data);

        accessor.with(|mut store| store.get().table.push(buf.into())).map_err(|e| e.to_string())
    }

    async fn write_data<T>(
        accessor: &Accessor<T, Self>, self_: Resource<ContainerProxy>, name: String,
        data: Resource<OutgoingValue>,
    ) -> Result<()> {
        let bytes = accessor
            .with(|mut store| {
                let value = store.get().table.get(&data)?;
                Ok::<Vec<u8>, wasmtime::Error>(value.pipe.contents().to_vec())
            })
            .map_err(|e| e.to_string())?;

        let container = get_container(accessor, &self_)?;
        container
            .write_data(name, bytes)
            .await
            .context("writing data")
            .map_err(|e| e.to_string())?;

        Ok(())
    }

    async fn list_objects<T>(
        accessor: &Accessor<T, Self>, self_: Resource<ContainerProxy>,
    ) -> Result<Resource<StreamObjectNames>> {
        let container = get_container(accessor, &self_)?;
        let names =
            container.list_objects().await.context("listing objects").map_err(|e| e.to_string())?;
        let stream = StreamObjectNames::new(names);
        accessor.with(|mut store| store.get().table.push(stream)).map_err(|e| e.to_string())
    }

    async fn delete_object<T>(
        accessor: &Accessor<T, Self>, self_: Resource<ContainerProxy>, name: String,
    ) -> Result<()> {
        let container = get_container(accessor, &self_)?;
        container.delete_object(name).await.context("deleting object").map_err(|e| e.to_string())
    }

    async fn delete_objects<T>(
        accessor: &Accessor<T, Self>, self_: Resource<ContainerProxy>, names: Vec<String>,
    ) -> Result<()> {
        let container = get_container(accessor, &self_)?;
        for name in names {
            container
                .delete_object(name)
                .await
                .context("deleting object")
                .map_err(|e| e.to_string())?;
        }

        Ok(())
    }

    async fn has_object<T>(
        accessor: &Accessor<T, Self>, self_: Resource<ContainerProxy>, name: String,
    ) -> Result<bool> {
        let container = get_container(accessor, &self_)?;
        container
            .has_object(name)
            .await
            .context("checking object exists")
            .map_err(|e| e.to_string())
    }

    async fn object_info<T>(
        accessor: &Accessor<T, Self>, self_: Resource<ContainerProxy>, name: String,
    ) -> Result<ObjectMetadata> {
        let container = get_container(accessor, &self_)?;
        container.object_info(name).await.context("getting object info").map_err(|e| e.to_string())
    }

    async fn clear<T>(accessor: &Accessor<T, Self>, self_: Resource<ContainerProxy>) -> Result<()> {
        let container = get_container(accessor, &self_)?;

        let all_objects =
            container.list_objects().await.context("listing objects").map_err(|e| e.to_string())?;

        for name in all_objects {
            container
                .delete_object(name)
                .await
                .context("deleting object")
                .map_err(|e| e.to_string())?;
        }

        Ok(())
    }

    fn drop<T>(
        mut accessor: Access<'_, T, Self>, rep: Resource<ContainerProxy>,
    ) -> wasmtime::Result<()> {
        Ok(accessor.get().table.delete(rep).map(|_| ())?)
    }
}

impl HostStreamObjectNamesWithStore for WasiBlobstore {
    async fn read_stream_object_names<T>(
        accessor: &Accessor<T, Self>, self_: Resource<StreamObjectNames>, len: u64,
    ) -> Result<(Vec<String>, bool)> {
        accessor.with(|mut store| {
            let stream = store
                .get()
                .table
                .get_mut(&self_)
                .context("StreamObjectNames not found")
                .map_err(|e| e.to_string())?;

            let remaining = &stream.names[stream.offset..];
            let take = usize::try_from(len).unwrap_or(usize::MAX).min(remaining.len());
            let batch = remaining[..take].to_vec();
            stream.offset += take;
            let done = stream.offset >= stream.names.len();
            Ok((batch, done))
        })
    }

    async fn skip_stream_object_names<T>(
        accessor: &Accessor<T, Self>, self_: Resource<StreamObjectNames>, num: u64,
    ) -> Result<(u64, bool)> {
        accessor.with(|mut store| {
            let stream = store
                .get()
                .table
                .get_mut(&self_)
                .context("StreamObjectNames not found")
                .map_err(|e| e.to_string())?;

            let remaining = stream.names.len() - stream.offset;
            let skip = usize::try_from(num).unwrap_or(usize::MAX).min(remaining);
            stream.offset += skip;
            let done = stream.offset >= stream.names.len();
            Ok((skip as u64, done))
        })
    }

    fn drop<T>(
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
        let container = store
            .get()
            .table
            .get(self_)
            .context("Container not found")
            .map_err(|e| e.to_string())?;
        Ok(container.clone())
    })
}
