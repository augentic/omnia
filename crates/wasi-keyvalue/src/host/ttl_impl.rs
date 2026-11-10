use std::time::Duration;

use anyhow::Context;
use omnia_core::HostView;
use wasmtime::component::{Accessor, Resource};

use crate::host::Result;
use crate::host::generated::omnia::keyvalue::ttl::{Host, HostWithStore};
use crate::host::resource::BucketProxy;
use crate::host::store_impl::get_bucket;
use crate::{KeyValueClient, WasiKeyValue};

impl<T> HostWithStore<T> for WasiKeyValue {
    async fn set(
        accessor: &Accessor<T, Self>, b: Resource<BucketProxy>, key: String, value: Vec<u8>,
        ttl_secs: u64,
    ) -> Result<()> {
        let bucket = get_bucket(accessor, &b)?;
        bucket
            .set(key, value, Some(Duration::from_secs(ttl_secs)))
            .await
            .context("issue setting value with ttl")?;
        Ok(())
    }
}

impl Host for HostView<'_, dyn KeyValueClient> {}
