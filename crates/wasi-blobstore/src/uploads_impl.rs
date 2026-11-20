use omnia_core::HostView;
use wasmtime::component::{Access, Accessor, FutureReader, Resource, StreamReader};

use crate::container_impl::ContainerHandle;
use crate::generated::omnia::blobstore::types::{Attributes, Conflict, Precondition, Stored};
use crate::generated::omnia::blobstore::uploads::{
    Host, HostUpload, HostUploadWithStore, HostWithStore, Part, Upload,
};
use crate::{BlobStoreClient, Error, Result, WasiBlobStore};

impl<T> HostWithStore<T> for WasiBlobStore {
    fn begin(
        _: &Accessor<T, Self>, _: Resource<ContainerHandle>, _: String, _: Attributes,
    ) -> impl Future<Output = Result<Resource<Upload>>> + Send {
        std::future::ready(Err(unsupported()))
    }

    fn resume(
        _: &Accessor<T, Self>, _: Resource<ContainerHandle>, _: String,
    ) -> impl Future<Output = Result<Resource<Upload>>> + Send {
        std::future::ready(Err(unsupported()))
    }
}

fn unsupported() -> Error {
    Error::Unsupported("this backend serves no multipart uploads".to_owned())
}

// no `upload` handle exists while `begin` and `resume` refuse every call
impl<T> HostUploadWithStore<T> for WasiBlobStore {
    fn id(_: Access<'_, T, Self>, _: Resource<Upload>) -> wasmtime::Result<String> {
        Err(unsupported().into())
    }

    fn put_part(
        mut host: Access<'_, T, Self>, _: Resource<Upload>, _: u32, _: u64,
        mut data: StreamReader<u8>,
    ) -> wasmtime::Result<FutureReader<Result<()>>> {
        data.close(&mut host)?;
        let reply = std::future::ready(Ok::<_, wasmtime::Error>(Err(unsupported())));
        FutureReader::new(&mut host, reply)
    }

    fn parts(
        _: &Accessor<T, Self>, _: Resource<Upload>,
    ) -> impl Future<Output = Result<Vec<Part>>> + Send {
        std::future::ready(Err(unsupported()))
    }

    fn commit(
        _: &Accessor<T, Self>, _: Resource<Upload>, _: Option<Precondition>,
    ) -> impl Future<Output = Result<std::result::Result<Stored, Conflict>>> + Send {
        std::future::ready(Err(unsupported()))
    }

    fn abort(
        _: &Accessor<T, Self>, _: Resource<Upload>,
    ) -> impl Future<Output = Result<()>> + Send {
        std::future::ready(Err(unsupported()))
    }

    fn drop(mut host: Access<'_, T, Self>, rep: Resource<Upload>) -> wasmtime::Result<()> {
        match host.get().table.delete(rep)? {}
    }
}

impl Host for HostView<'_, dyn BlobStoreClient> {}

impl HostUpload for HostView<'_, dyn BlobStoreClient> {}
