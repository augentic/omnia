use omnia_core::HostView;
use wasmtime::component::{Accessor, Resource};

use crate::bucket_impl::BucketHandle;
use crate::generated::omnia::keyvalue::keys::{Host, HostWithStore, Page};
use crate::{Error, KeyValueClient, Result, WasiKeyValue};

const PAGE_LIMIT: u32 = 1_000;

impl<T> HostWithStore<T> for WasiKeyValue {
    async fn list(
        accessor: &Accessor<T, Self>, bucket: Resource<BucketHandle>, prefix: String,
        after: Option<String>, limit: Option<u32>,
    ) -> Result<Page> {
        let limit = match limit {
            Some(0) => return Err(Error::Invalid("a listing limit must be at least 1".to_owned())),
            Some(limit) => limit.min(PAGE_LIMIT) as usize,
            None => PAGE_LIMIT as usize,
        };
        let keys = BucketHandle::lookup(accessor, &bucket)?.keys().await?;
        Ok(page(keys, &prefix, after.as_deref(), limit))
    }
}

// A backend lists every key at once, so the host filters, orders, and pages.
fn page(mut keys: Vec<String>, prefix: &str, after: Option<&str>, limit: usize) -> Page {
    keys.retain(|key| key.starts_with(prefix) && after.is_none_or(|after| key.as_str() > after));
    keys.sort_unstable();
    let next = (keys.len() > limit).then(|| keys[limit - 1].clone());
    keys.truncate(limit);
    Page { keys, next }
}

impl Host for HostView<'_, dyn KeyValueClient> {}
