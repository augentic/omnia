use wasmtime::component::{Access, Accessor, Resource};
use wasmtime::error::Context;
use wasmtime_wasi::p2::bindings::io::streams::{InputStream, OutputStream};
use wasmtime_wasi::p2::pipe::MemoryInputPipe;

use crate::host::generated::wasi::blobstore::types::{
    Host, HostIncomingValue, HostIncomingValueWithStore, HostOutgoingValue,
    HostOutgoingValueWithStore, IncomingValueSyncBody,
};
use crate::host::{
    Error, IncomingValue, OutgoingValue, Result, WasiBlobstore, WasiBlobstoreCtxView,
};

impl<T> HostIncomingValueWithStore<T> for WasiBlobstore {
    fn incoming_value_consume_sync(
        mut host: Access<'_, T, Self>, this: Resource<IncomingValue>,
    ) -> Result<IncomingValueSyncBody> {
        let value = host.get().table.get(&this).context("IncomingValue not found")?.to_vec();
        Ok(value)
    }

    fn incoming_value_consume_async(
        accessor: &Accessor<T, Self>, this: Resource<IncomingValue>,
    ) -> impl std::future::Future<Output = Result<Resource<InputStream>>> {
        std::future::ready(accessor.with(|mut store| {
            let incoming = store.get().table.get(&this).context("IncomingValue not found")?;
            let rs = MemoryInputPipe::new(incoming.clone());
            let stream: InputStream = Box::new(rs);
            Ok(store.get().table.push(stream)?)
        }))
    }

    fn size(
        mut host: Access<'_, T, Self>, self_: Resource<IncomingValue>,
    ) -> wasmtime::Result<u64> {
        let value = host.get().table.get(&self_).context("IncomingValue not found")?;
        Ok(value.len() as u64)
    }

    fn drop(
        mut accessor: Access<'_, T, Self>, rep: Resource<IncomingValue>,
    ) -> wasmtime::Result<()> {
        Ok(accessor.get().table.delete(rep).map(|_| ())?)
    }
}

impl<T> HostOutgoingValueWithStore<T> for WasiBlobstore {
    fn new_outgoing_value(
        mut host: Access<'_, T, Self>,
    ) -> wasmtime::Result<Resource<OutgoingValue>> {
        // The pipe is never drained (`finish` hands the whole buffer to the
        // backend), so a finite capacity only traps writes past it; the
        // buffer grows on demand from empty.
        Ok(host.get().table.push(OutgoingValue::new(usize::MAX))?)
    }

    fn outgoing_value_write_body(
        accessor: &wasmtime::component::Accessor<T, Self>,
        self_: wasmtime::component::Resource<OutgoingValue>,
    ) -> impl std::future::Future<
        Output = wasmtime::Result<
            wasmtime::Result<wasmtime::component::Resource<OutputStream>, ()>,
        >,
    > {
        std::future::ready(accessor.with(|mut store| {
            let pipe = {
                let outgoing =
                    store.get().table.get_mut(&self_).context("OutgoingValue not found")?;
                if outgoing.take_write_body().is_err() {
                    return Ok(Err(()));
                }
                outgoing.pipe.clone()
            };

            let stream: OutputStream = Box::new(pipe);
            let stream_resource = store.get().table.push(stream)?;
            Ok(Ok(stream_resource))
        }))
    }

    fn finish(mut host: Access<'_, T, Self>, this: Resource<OutgoingValue>) -> Result<()> {
        let outgoing = host.get().table.get_mut(&this).context("OutgoingValue not found")?;

        outgoing.finalize().map_err(|msg| Error::Other(msg.to_string()))
    }

    fn drop(
        mut accessor: Access<'_, T, Self>, rep: Resource<OutgoingValue>,
    ) -> wasmtime::Result<()> {
        Ok(accessor.get().table.delete(rep).map(|_| ())?)
    }
}

impl Host for WasiBlobstoreCtxView<'_> {
    fn convert_error(&mut self, err: Error) -> wasmtime::Result<String> {
        Ok(err.to_string())
    }
}
impl HostIncomingValue for WasiBlobstoreCtxView<'_> {}
impl HostOutgoingValue for WasiBlobstoreCtxView<'_> {}
