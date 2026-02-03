use bytes::Bytes;
use wasmtime::component::{Access, Accessor, Resource};
use wasmtime::error::Context;
use wasmtime_wasi::p2::bindings::io::streams::{InputStream, OutputStream};
use wasmtime_wasi::p2::pipe::{MemoryInputPipe, MemoryOutputPipe};

use crate::host::generated::Error;
use crate::host::generated::wasi::blobstore::types::{
    Host, HostIncomingValue, HostIncomingValueWithStore, HostOutgoingValue,
    HostOutgoingValueWithStore, IncomingValueSyncBody,
};
use crate::host::{Result, WasiBlobstore, WasiBlobstoreCtxView};

pub type IncomingValue = Bytes;
pub type OutgoingValue = MemoryOutputPipe;

impl HostIncomingValueWithStore for WasiBlobstore {
    fn incoming_value_consume_sync<T>(
        mut host: Access<'_, T, Self>, this: Resource<IncomingValue>,
    ) -> Result<IncomingValueSyncBody> {
        let value = host
            .get()
            .table
            .get(&this)
            .context("IncomingValue not found")
            .map_err(|e| e.to_string())?
            .to_vec();
        Ok(value)
    }

    async fn incoming_value_consume_async<T>(
        accessor: &Accessor<T, Self>, this: Resource<IncomingValue>,
    ) -> Result<Resource<InputStream>> {
        let value = accessor
            .with(|mut store| {
                let incoming = store.get().table.get(&this).context("IncomingValue not found")?;
                Ok::<bytes::Bytes, wasmtime::Error>(incoming.clone())
            })
            .map_err(|e| e.to_string())?;
        let rs = MemoryInputPipe::new(value);
        let stream: InputStream = Box::new(rs);
        accessor.with(|mut store| store.get().table.push(stream)).map_err(|e| e.to_string())
    }

    fn size<T>(
        mut host: Access<'_, T, Self>, self_: Resource<IncomingValue>,
    ) -> wasmtime::Result<u64> {
        let value = host.get().table.get(&self_).context("IncomingValue not found")?;
        Ok(value.len() as u64)
    }

    fn drop<T>(
        mut accessor: Access<'_, T, Self>, rep: Resource<IncomingValue>,
    ) -> wasmtime::Result<()> {
        Ok(accessor.get().table.delete(rep).map(|_| ())?)
    }
}

impl HostOutgoingValueWithStore for WasiBlobstore {
    fn new_outgoing_value<T>(
        mut host: Access<'_, T, Self>,
    ) -> wasmtime::Result<Resource<OutgoingValue>> {
        Ok(host.get().table.push(OutgoingValue::new(1024))?)
    }

    async fn outgoing_value_write_body<T>(
        accessor: &wasmtime::component::Accessor<T, Self>,
        self_: wasmtime::component::Resource<OutgoingValue>,
    ) -> wasmtime::Result<wasmtime::Result<wasmtime::component::Resource<OutputStream>, ()>> {
        let value = accessor.with(|mut store| {
            let outgoing = store.get().table.get(&self_).context("OutgoingValue not found")?;
            Ok::<_, wasmtime::Error>(outgoing.clone())
        })?;
        let stream: OutputStream = Box::new(value);
        Ok(accessor.with(|mut store| {
            store.get().table.push(stream).map_err(|e| {
                tracing::error!("Failed to fetch stream with error {e}");
            })
        }))
    }

    fn finish<T>(_: Access<'_, T, Self>, _self_: Resource<OutgoingValue>) -> Result<()> {
        Ok(())
    }

    fn drop<T>(
        mut accessor: Access<'_, T, Self>, rep: Resource<OutgoingValue>,
    ) -> wasmtime::Result<()> {
        Ok(accessor.get().table.delete(rep).map(|_| ())?)
    }
}

impl Host for WasiBlobstoreCtxView<'_> {
    fn convert_error(&mut self, err: Error) -> wasmtime::Result<Error> {
        Ok(err)
    }
}
impl HostIncomingValue for WasiBlobstoreCtxView<'_> {}
impl HostOutgoingValue for WasiBlobstoreCtxView<'_> {}
