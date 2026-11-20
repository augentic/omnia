use omnia_core::HostView;
use wasmtime::component::{Accessor, Resource};

use crate::host::bucket_impl::BucketHandle;
use crate::host::generated::omnia::keyvalue::store::{Host, HostWithStore};
use crate::host::{KeyValueClient, Result, WasiKeyValue};

impl<T> HostWithStore<T> for WasiKeyValue {
    async fn open(accessor: &Accessor<T, Self>, name: String) -> Result<Resource<BucketHandle>> {
        let bucket =
            accessor.with(|mut store| store.get().client.open_bucket(name.clone())).await?;
        BucketHandle::push(accessor, name, bucket)
    }
}

impl Host for HostView<'_, dyn KeyValueClient> {}
