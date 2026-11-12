use std::sync::Arc;
use std::time::Duration;

use omnia_core::HostView;
use wasmtime::component::Accessor;

use crate::host::generated::omnia::keyvalue::store::{Conflict, Host, HostWithStore};
use crate::host::resource::Bucket;
use crate::host::{Error, KeyValueClient, Result, WasiKeyValue};

impl<T> HostWithStore<T> for WasiKeyValue {
    async fn get(
        accessor: &Accessor<T, Self>, bucket: String, key: String,
    ) -> Result<Option<Vec<u8>>> {
        Ok(open(accessor, bucket).await?.get(key).await?)
    }

    async fn set(
        accessor: &Accessor<T, Self>, bucket: String, key: String, value: Vec<u8>,
        ttl_ms: Option<u64>,
    ) -> Result<()> {
        let ttl = ttl(ttl_ms)?;
        Ok(open(accessor, bucket).await?.set(key, value, ttl).await?)
    }

    async fn delete(accessor: &Accessor<T, Self>, bucket: String, key: String) -> Result<()> {
        Ok(open(accessor, bucket).await?.delete(key).await?)
    }

    async fn compare_and_swap(
        accessor: &Accessor<T, Self>, bucket: String, key: String, current: Option<Vec<u8>>,
        new: Vec<u8>, ttl_ms: Option<u64>,
    ) -> Result<std::result::Result<(), Conflict>> {
        let ttl = ttl(ttl_ms)?;
        let swapped =
            open(accessor, bucket).await?.compare_and_swap(key, current, new, ttl).await?;
        Ok(swapped.map_err(|current| Conflict { current }))
    }

    async fn increment(
        accessor: &Accessor<T, Self>, bucket: String, key: String, delta: i64, ttl_ms: Option<u64>,
    ) -> Result<i64> {
        let ttl = ttl(ttl_ms)?;
        Ok(open(accessor, bucket).await?.increment(key, delta, ttl).await?)
    }
}

async fn open<T>(
    accessor: &Accessor<T, WasiKeyValue>, bucket: String,
) -> anyhow::Result<Arc<dyn Bucket>> {
    accessor.with(|mut store| store.get().client.open_bucket(bucket)).await
}

// a zero TTL would expire the entry as it is written, so it writes nothing
fn ttl(ttl_ms: Option<u64>) -> Result<Option<Duration>> {
    match ttl_ms {
        Some(0) => Err(Error::Invalid("a TTL must be greater than zero".to_owned())),
        ttl_ms => Ok(ttl_ms.map(Duration::from_millis)),
    }
}

impl Host for HostView<'_, dyn KeyValueClient> {
    fn convert_error(&mut self, err: Error) -> wasmtime::Result<Error> {
        Ok(err)
    }
}
