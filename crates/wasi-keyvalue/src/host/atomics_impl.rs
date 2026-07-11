use anyhow::Context;
use wasmtime::component::{Access, Accessor, Resource};

use crate::WasiKeyValueCtxView;
use crate::host::generated::wasi::keyvalue::atomics::{
    CasError, Host, HostCas, HostCasWithStore, HostWithStore,
};
use crate::host::generated::wasi::keyvalue::store::Error;
use crate::host::resource::{BucketProxy, Cas};
use crate::host::store_impl::get_bucket;
use crate::host::{Result, WasiKeyValue};

impl<T> HostWithStore<T> for WasiKeyValue {
    /// Atomically increment the value associated with the key in the store by
    /// the given delta. It returns the new value.
    ///
    /// If the key does not exist in the store, it creates a new key-value pair
    /// with the value set to the given delta.
    ///
    /// If any other error occurs, it returns an `Err(error)`.
    async fn increment(
        accessor: &Accessor<T, Self>, bucket: Resource<BucketProxy>, key: String, delta: i64,
    ) -> Result<i64> {
        let bucket = get_bucket(accessor, &bucket)?;

        // A missing key starts from zero, so the increment creates it at `delta`.
        let base =
            bucket.get(key.clone()).await.context("issue getting value")?.map_or(0, |value| {
                let mut buf = [0u8; 8];
                let len = 8.min(value.len());
                buf[..len].copy_from_slice(&value[..len]);
                i64::from_be_bytes(buf)
            });
        let inc = base + delta;

        bucket.set(key, inc.to_be_bytes().to_vec()).await.context("issue saving increment")?;
        Ok(inc)
    }

    /// Perform the swap on a CAS operation. This consumes the CAS handle and
    /// returns an error if the CAS operation failed.
    ///
    /// The default is read-compare-set on the [`crate::Bucket`] trait; backends
    /// with a native compare-and-swap primitive can tighten the race window by
    /// versioning inside their `Bucket` implementation.
    async fn swap(
        accessor: &Accessor<T, Self>, cas: Resource<Cas>, value: Vec<u8>,
    ) -> anyhow::Result<anyhow::Result<(), CasError>, wasmtime::Error> {
        // The WIT consumes the handle, so remove it from the table up front.
        let cas = accessor.with(|mut store| store.get().table.delete(cas))?;

        let observed = match cas.bucket.get(cas.key.clone()).await {
            Ok(observed) => observed,
            Err(error) => return Ok(Err(CasError::StoreError(Error::from(error)))),
        };
        if observed != cas.current {
            // Stale snapshot: hand back a fresh handle at the latest value so
            // the guest can retry, as the WIT contract requires.
            let fresh = Cas {
                bucket: cas.bucket,
                key: cas.key,
                current: observed,
            };
            let resource = accessor.with(|mut store| store.get().table.push(fresh))?;
            return Ok(Err(CasError::CasFailed(resource)));
        }

        match cas.bucket.set(cas.key, value).await {
            Ok(()) => Ok(Ok(())),
            Err(error) => Ok(Err(CasError::StoreError(Error::from(error)))),
        }
    }
}

impl<T> HostCasWithStore<T> for WasiKeyValue {
    /// Construct a new CAS operation. Implementors can map the underlying functionality
    /// (transactions, versions, etc) as desired.
    async fn new(
        accessor: &Accessor<T, Self>, bucket: Resource<BucketProxy>, key: String,
    ) -> Result<Resource<Cas>> {
        let bucket = get_bucket(accessor, &bucket)?;
        let current = bucket.get(key.clone()).await.context("issue getting key")?;
        let cas = Cas {
            bucket: bucket.0,
            key,
            current,
        };
        Ok(accessor.with(|mut store| store.get().table.push(cas))?)
    }

    /// Get the current value of the CAS handle.
    async fn current(
        accessor: &Accessor<T, Self>, self_: Resource<Cas>,
    ) -> Result<Option<Vec<u8>>> {
        let cas = accessor.with(|mut store| {
            let cas = store.get().table.get(&self_).map_err(|_e| Error::NoSuchStore)?;
            Ok::<_, Error>(cas.clone())
        })?;
        Ok(cas.current)
    }

    /// Drop the CAS handle.
    fn drop(mut accessor: Access<'_, T, Self>, rep: Resource<Cas>) -> wasmtime::Result<()> {
        tracing::trace!("atomics::HostCas::drop");
        Ok(accessor.get().table.delete(rep).map(|_| ())?)
    }
}

impl Host for WasiKeyValueCtxView<'_> {}
impl HostCas for WasiKeyValueCtxView<'_> {}
