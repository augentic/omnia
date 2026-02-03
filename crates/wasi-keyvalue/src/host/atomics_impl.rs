use anyhow::{Context, anyhow};
use wasmtime::component::{Access, Accessor, Resource};

use crate::WasiKeyValueCtxView;
use crate::host::generated::wasi::keyvalue::atomics::{
    CasError, Host, HostCas, HostCasWithStore, HostWithStore,
};
use crate::host::generated::wasi::keyvalue::store::Error;
use crate::host::resource::{BucketProxy, Cas};
use crate::host::store_impl::get_bucket;
use crate::host::{Result, WasiKeyValue};

impl HostWithStore for WasiKeyValue {
    /// Atomically increment the value associated with the key in the store by
    /// the given delta. It returns the new value.
    ///
    /// If the key does not exist in the store, it creates a new key-value pair
    /// with the value set to the given delta.
    ///
    /// If any other error occurs, it returns an `Err(error)`.
    async fn increment<T>(
        accessor: &Accessor<T, Self>, bucket: Resource<BucketProxy>, key: String, delta: i64,
    ) -> Result<i64> {
        let bucket = get_bucket(accessor, &bucket)?;

        let Ok(Some(value)) = bucket.get(key.clone()).await else {
            return Err(anyhow!("no value for {key}").into());
        };

        // increment value by delta
        let slice: &[u8] = &value;
        let mut buf = [0u8; 8];
        let len = 8.min(slice.len());
        buf[..len].copy_from_slice(&slice[..len]);
        let inc = i64::from_be_bytes(buf) + delta;

        // update value in bucket
        if let Err(e) = bucket.set(key, inc.to_be_bytes().to_vec()).await {
            return Err(anyhow!("issue saving increment: {e}").into());
        }

        Ok(inc)
    }

    /// Perform the swap on a CAS operation. This consumes the CAS handle and
    /// returns an error if the CAS operation failed.
    async fn swap<T>(
        _store: &Accessor<T, Self>, _self_: Resource<Cas>, _value: Vec<u8>,
    ) -> anyhow::Result<Result<(), CasError>, wasmtime::Error> {
        Err(wasmtime::Error::msg("not implemented"))
    }
}

impl HostCasWithStore for WasiKeyValue {
    /// Construct a new CAS operation. Implementors can map the underlying functionality
    /// (transactions, versions, etc) as desired.
    async fn new<T>(
        accessor: &Accessor<T, Self>, bucket: Resource<BucketProxy>, key: String,
    ) -> Result<Resource<Cas>> {
        let bucket = get_bucket(accessor, &bucket)?;
        let current = bucket.get(key.clone()).await.context("issue getting key")?;
        let cas = Cas { key, current };
        Ok(accessor.with(|mut store| store.get().table.push(cas))?)
    }

    /// Get the current value of the CAS handle.
    async fn current<T>(
        accessor: &Accessor<T, Self>, self_: Resource<Cas>,
    ) -> Result<Option<Vec<u8>>> {
        let cas = accessor.with(|mut store| {
            let cas = store.get().table.get(&self_).map_err(|_e| Error::NoSuchStore)?;
            Ok::<_, Error>(cas.clone())
        })?;
        Ok(cas.current)
    }

    /// Drop the CAS handle.
    fn drop<T>(mut accessor: Access<'_, T, Self>, rep: Resource<Cas>) -> wasmtime::Result<()> {
        tracing::trace!("atomics::HostCas::drop");
        Ok(accessor.get().table.delete(rep).map(|_| ())?)
    }
}

impl Host for WasiKeyValueCtxView<'_> {}
impl HostCas for WasiKeyValueCtxView<'_> {}
