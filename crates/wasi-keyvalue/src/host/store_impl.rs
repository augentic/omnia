use anyhow::Context;
use wasmtime::component::{Access, Accessor, Resource};

use crate::host::generated::wasi::keyvalue::store::{
    Error, HostBucketWithStore, HostWithStore, KeyResponse,
};
use crate::host::resource::BucketProxy;
use crate::host::store::{Host, HostBucket};
use crate::host::{Result, WasiKeyValue, WasiKeyValueCtxView};

impl HostWithStore for WasiKeyValue {
    async fn open<T>(
        accessor: &Accessor<T, Self>, identifier: String,
    ) -> Result<Resource<BucketProxy>> {
        let bucket = accessor.with(|mut store| store.get().ctx.open_bucket(identifier)).await?;
        let proxy = BucketProxy(bucket);
        Ok(accessor.with(|mut store| store.get().table.push(proxy))?)
    }
}

impl HostBucketWithStore for WasiKeyValue {
    async fn get<T>(
        accessor: &Accessor<T, Self>, self_: Resource<BucketProxy>, key: String,
    ) -> Result<Option<Vec<u8>>> {
        let bucket = get_bucket(accessor, &self_)?;
        let value = bucket.get(key).await.context("issue getting value")?;
        Ok(value)
    }

    async fn set<T>(
        accessor: &Accessor<T, Self>, self_: Resource<BucketProxy>, key: String, value: Vec<u8>,
    ) -> Result<()> {
        let bucket = get_bucket(accessor, &self_)?;
        bucket.set(key, value).await.context("issue setting value")?;
        Ok(())
    }

    async fn delete<T>(
        accessor: &Accessor<T, Self>, self_: Resource<BucketProxy>, key: String,
    ) -> Result<()> {
        let bucket = get_bucket(accessor, &self_)?;
        bucket.delete(key).await.context("issue deleting value")?;
        Ok(())
    }

    async fn exists<T>(
        accessor: &Accessor<T, Self>, self_: Resource<BucketProxy>, key: String,
    ) -> Result<bool> {
        let bucket = get_bucket(accessor, &self_)?;
        let value = bucket.get(key).await.context("issue getting value")?;
        Ok(value.is_some())
    }

    async fn list_keys<T>(
        accessor: &Accessor<T, Self>, self_: Resource<BucketProxy>, cursor: Option<String>,
    ) -> Result<KeyResponse> {
        tracing::trace!("store::HostBucket::list_keys {cursor:?}");
        let bucket = get_bucket(accessor, &self_)?;
        let keys = bucket.keys().await.context("issue getting value")?;
        Ok(KeyResponse { keys, cursor })
    }

    fn drop<T>(
        mut accessor: Access<'_, T, Self>, rep: Resource<BucketProxy>,
    ) -> std::result::Result<(), wasmtime::Error> {
        Ok(accessor.get().table.delete(rep).map(|_| ())?)
    }
}

impl Host for WasiKeyValueCtxView<'_> {
    fn convert_error(&mut self, err: Error) -> wasmtime::Result<Error> {
        Ok(err)
    }
}

impl HostBucket for WasiKeyValueCtxView<'_> {}

pub fn get_bucket<T>(
    accessor: &Accessor<T, WasiKeyValue>, self_: &Resource<BucketProxy>,
) -> Result<BucketProxy> {
    accessor.with(|mut store| {
        let bucket = store.get().table.get(self_).map_err(|_e| Error::NoSuchStore)?;
        Ok::<_, Error>(bucket.clone())
    })
}
