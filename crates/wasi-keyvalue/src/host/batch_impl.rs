use anyhow::anyhow;
use wasmtime::component::{Accessor, Resource};

use crate::WasiKeyValueCtxView;
use crate::host::generated::wasi::keyvalue::batch::{Host, HostWithStore};
use crate::host::resource::BucketProxy;
use crate::host::store_impl::get_bucket;
use crate::host::{Result, WasiKeyValue};

impl<T> HostWithStore<T> for WasiKeyValue {
    async fn get_many(
        accessor: &Accessor<T, Self>, bucket: Resource<BucketProxy>, keys: Vec<String>,
    ) -> Result<Vec<Option<(String, Vec<u8>)>>> {
        let bucket = get_bucket(accessor, &bucket)?;

        // The WIT contract returns one entry per requested key, positionally
        // aligned: `some((key, value))` when present, `none` when absent.
        let mut many = Vec::with_capacity(keys.len());
        for key in keys {
            let entry = bucket.get(key.clone()).await?.map(|value| (key, value));
            many.push(entry);
        }

        Ok(many)
    }

    async fn set_many(
        accessor: &Accessor<T, Self>, bucket: Resource<BucketProxy>,
        key_values: Vec<(String, Vec<u8>)>,
    ) -> Result<()> {
        let bucket = get_bucket(accessor, &bucket)?;
        for (key, value) in key_values {
            bucket.set(key, value).await?;
        }
        Ok(())
    }

    async fn delete_many(
        accessor: &Accessor<T, Self>, bucket: Resource<BucketProxy>, keys: Vec<String>,
    ) -> Result<()> {
        let bucket = get_bucket(accessor, &bucket)?;
        for key in keys {
            if let Err(e) = bucket.delete(key).await {
                return Err(anyhow!("issue deleting value: {e}").into());
            }
        }
        Ok(())
    }
}

impl Host for WasiKeyValueCtxView<'_> {}
