use omnia_core::HostView;
use wasmtime::component::{Accessor, Resource};

use crate::collection_impl::CollectionHandle;
use crate::generated::omnia::docstore::store::{Host, HostWithStore};
use crate::{DocStoreClient, Result, WasiDocStore};

impl<T> HostWithStore<T> for WasiDocStore {
    // `DocStoreClient` has no call that opens a collection, so a name the
    // deployment does not declare fails at the handle's first call
    fn open(
        accessor: &Accessor<T, Self>, name: String,
    ) -> impl Future<Output = Result<Resource<CollectionHandle>>> + Send {
        std::future::ready(CollectionHandle::push(accessor, name))
    }
}

impl Host for HostView<'_, dyn DocStoreClient> {}
