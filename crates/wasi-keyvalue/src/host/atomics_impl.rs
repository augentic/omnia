use std::future::{self, Future};
use std::sync::Arc;

use anyhow::Context;
use omnia_core::HostView;
use wasmtime::component::{Access, Accessor, Resource};

use crate::KeyValueClient;
use crate::host::generated::WasiError;
use crate::host::generated::wasi::keyvalue::atomics::{
    CasError, Host, HostCas, HostCasWithStore, HostWithStore,
};
use crate::host::resource::{BucketProxy, Cas};
use crate::host::store_impl::get_bucket;
use crate::host::{WasiKeyValue, WasiResult};

impl<T> HostWithStore<T> for WasiKeyValue {
    async fn increment(
        accessor: &Accessor<T, Self>, bucket: Resource<BucketProxy>, key: String, delta: i64,
    ) -> WasiResult<i64> {
        let bucket = get_bucket(accessor, &bucket)?;
        Ok(bucket.increment(key, delta, None).await.context("issue incrementing value")?)
    }

    async fn swap(
        accessor: &Accessor<T, Self>, cas: Resource<Cas>, value: Vec<u8>,
    ) -> std::result::Result<(), CasError> {
        let cas = accessor.with(|mut store| store.get().table.delete(cas))?;
        let bucket = Arc::clone(&cas.bucket);

        match bucket.compare_and_swap(cas.key.clone(), cas.current.clone(), value, None).await {
            Ok(Ok(())) => Ok(()),
            Ok(Err(observed)) => {
                let fresh = Cas {
                    current: observed,
                    ..cas
                };
                let resource = accessor.with(|mut store| store.get().table.push(fresh))?;
                Err(CasError::CasFailed(resource))
            }
            Err(error) => Err(error.into()),
        }
    }
}

impl<T> HostCasWithStore<T> for WasiKeyValue {
    async fn new(
        accessor: &Accessor<T, Self>, bucket: Resource<BucketProxy>, key: String,
    ) -> WasiResult<Resource<Cas>> {
        let bucket = get_bucket(accessor, &bucket)?;
        let current = bucket.get(key.clone()).await.context("issue getting key")?;
        let cas = Cas {
            bucket: bucket.0,
            key,
            current,
        };
        Ok(accessor.with(|mut store| store.get().table.push(cas))?)
    }

    fn current(
        accessor: &Accessor<T, Self>, self_: Resource<Cas>,
    ) -> impl Future<Output = WasiResult<Option<Vec<u8>>>> {
        future::ready(accessor.with(|mut store| {
            store
                .get()
                .table
                .get(&self_)
                .map(|cas| cas.current.clone())
                .map_err(|_stale| WasiError::NoSuchStore)
        }))
    }

    fn drop(mut accessor: Access<'_, T, Self>, rep: Resource<Cas>) -> wasmtime::Result<()> {
        Ok(accessor.get().table.delete(rep).map(|_| ())?)
    }
}

impl From<anyhow::Error> for CasError {
    fn from(err: anyhow::Error) -> Self {
        Self::StoreError(WasiError::from(err))
    }
}

impl From<wasmtime::component::ResourceTableError> for CasError {
    fn from(err: wasmtime::component::ResourceTableError) -> Self {
        Self::StoreError(WasiError::from(err))
    }
}

impl Host for HostView<'_, dyn KeyValueClient> {
    fn convert_cas_error(&mut self, err: CasError) -> wasmtime::Result<CasError> {
        Ok(err)
    }
}
impl HostCas for HostView<'_, dyn KeyValueClient> {}
