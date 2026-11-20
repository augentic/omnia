use omnia_core::HostView;
use wasmtime::component::{Accessor, Resource};

use crate::host::container_impl::ContainerHandle;
use crate::host::generated::omnia::blobstore::presign::{Host, HostWithStore, PresignedRequest};
use crate::host::generated::omnia::blobstore::types::{Attributes, Precondition};
use crate::host::{BlobStoreClient, Error, Result, WasiBlobStore};

impl<T> HostWithStore<T> for WasiBlobStore {
    fn read(
        _: &Accessor<T, Self>, _: Resource<ContainerHandle>, _: String, _: u64,
    ) -> impl Future<Output = Result<PresignedRequest>> + Send {
        std::future::ready(Err(unsupported()))
    }

    fn write(
        _: &Accessor<T, Self>, _: Resource<ContainerHandle>, _: String, _: Attributes,
        _: Option<Precondition>, _: u64,
    ) -> impl Future<Output = Result<PresignedRequest>> + Send {
        std::future::ready(Err(unsupported()))
    }
}

fn unsupported() -> Error {
    Error::Unsupported("this backend issues no presigned requests".to_owned())
}

impl Host for HostView<'_, dyn BlobStoreClient> {}
