use std::sync::Arc;

use wasmtime::component::{Access, Accessor, Resource};

use crate::host::generated::wasi::messaging::types;
pub use crate::host::generated::wasi::messaging::types::{
    Error, Host, HostClient, HostClientWithStore, HostMessage, HostMessageWithStore, Topic,
};
use crate::host::resource::{ClientProxy, MessageProxy};
use crate::host::{Result, WasiMessaging, WasiMessagingCtxView};

impl HostClientWithStore for WasiMessaging {
    async fn connect<T>(
        accessor: &Accessor<T, Self>, _name: String,
    ) -> Result<Resource<ClientProxy>> {
        let client = accessor.with(|mut store| store.get().ctx.connect()).await?;
        let proxy = ClientProxy(client);
        Ok(accessor.with(|mut store| store.get().table.push(proxy))?)
    }

    fn disconnect<T>(_: Access<'_, T, Self>, _: Resource<ClientProxy>) -> Result<()> {
        Ok(())
    }

    fn drop<T>(
        mut accessor: Access<'_, T, Self>, rep: Resource<ClientProxy>,
    ) -> wasmtime::Result<()> {
        Ok(accessor.get().table.delete(rep).map(|_| ())?)
    }
}

impl HostMessageWithStore for WasiMessaging {
    /// Create a new message with the given payload.
    fn new<T>(
        mut host: Access<'_, T, Self>, data: Vec<u8>,
    ) -> wasmtime::Result<Resource<MessageProxy>> {
        let message = host.get().ctx.new_message(data).map_err(wasmtime::Error::from_anyhow)?;
        let proxy = MessageProxy(message);
        Ok(host.get().table.push(proxy)?)
    }

    /// The topic/subject/channel this message was received on, if any.
    fn topic<T>(
        mut host: Access<'_, T, Self>, self_: Resource<MessageProxy>,
    ) -> wasmtime::Result<Option<Topic>> {
        let message = host.get().table.get(&self_)?;
        let topic = message.topic();
        if topic.is_empty() { Ok(None) } else { Ok(Some(topic)) }
    }

    /// An optional content-type describing the format of the data in the
    /// message. This is sometimes described as the "format" type".
    fn content_type<T>(
        mut host: Access<'_, T, Self>, self_: Resource<MessageProxy>,
    ) -> wasmtime::Result<Option<String>> {
        let message = host.get().table.get(&self_)?;
        if let Some(md) = message.metadata() {
            if let Some(content_type) = md.get("content-type") {
                return Ok(Some(content_type.clone()));
            }
            return Ok(None);
        }
        Ok(None)
    }

    /// Set the content-type describing the format of the data in the message.
    /// This is sometimes described as the "format" type.
    fn set_content_type<T>(
        mut host: Access<'_, T, Self>, self_: Resource<MessageProxy>, content_type: String,
    ) -> wasmtime::Result<()> {
        let store = host.get();
        let message = store.table.get(&self_)?;
        let updated_message = store
            .ctx
            .set_content_type(Arc::clone(&message.0), content_type)
            .map_err(wasmtime::Error::from_anyhow)?;
        store.table.push(updated_message)?;
        Ok(())
    }

    /// An opaque blob of data.
    fn data<T>(
        mut host: Access<'_, T, Self>, self_: Resource<MessageProxy>,
    ) -> wasmtime::Result<Vec<u8>> {
        let message = host.get().table.get(&self_)?;
        Ok(message.payload())
    }

    /// Set the opaque blob of data for this message, discarding the old value.
    fn set_data<T>(
        mut host: Access<'_, T, Self>, self_: Resource<MessageProxy>, data: Vec<u8>,
    ) -> wasmtime::Result<()> {
        let store = host.get();
        let message = store.table.get(&self_)?;
        let updated_message = store
            .ctx
            .set_payload(Arc::clone(&message.0), data)
            .map_err(wasmtime::Error::from_anyhow)?;
        store.table.push(updated_message)?;
        Ok(())
    }

    /// Get the metadata associated with this message.    
    fn metadata<T>(
        mut host: Access<'_, T, Self>, self_: Resource<MessageProxy>,
    ) -> wasmtime::Result<Option<types::Metadata>> {
        let message = host.get().table.get(&self_)?;
        if let Some(md) = message.metadata() {
            return Ok(Some(md.into()));
        }
        Ok(None)
    }

    /// Append a key-value pair to the metadata of this message.
    fn add_metadata<T>(
        mut host: Access<'_, T, Self>, self_: Resource<MessageProxy>, key: String, value: String,
    ) -> wasmtime::Result<()> {
        let store = host.get();
        let message = store.table.get(&self_)?;
        let updated_message = store
            .ctx
            .add_metadata(Arc::clone(&message.0), key, value)
            .map_err(wasmtime::Error::from_anyhow)?;
        store.table.push(updated_message)?;
        Ok(())
    }

    /// Set all the metadata on this message, replacing any existing metadata.
    fn set_metadata<T>(
        mut host: Access<'_, T, Self>, self_: Resource<MessageProxy>, meta: types::Metadata,
    ) -> wasmtime::Result<()> {
        let store = host.get();
        let message = store.table.get(&self_)?;
        let updated_message = store
            .ctx
            .set_metadata(Arc::clone(&message.0), meta.into())
            .map_err(wasmtime::Error::from_anyhow)?;
        store.table.push(updated_message)?;
        Ok(())
    }

    /// Remove a key-value pair from the metadata of a message.
    fn remove_metadata<T>(
        mut host: Access<'_, T, Self>, self_: Resource<MessageProxy>, key: String,
    ) -> wasmtime::Result<()> {
        let store = host.get();
        let message = store.table.get(&self_)?;
        let updated_message = store
            .ctx
            .remove_metadata(Arc::clone(&message.0), key)
            .map_err(wasmtime::Error::from_anyhow)?;
        store.table.push(updated_message)?;
        Ok(())
    }

    fn drop<T>(
        mut accessor: Access<'_, T, Self>, rep: Resource<MessageProxy>,
    ) -> wasmtime::Result<()> {
        Ok(accessor.get().table.delete(rep).map(|_| ())?)
    }
}

impl Host for WasiMessagingCtxView<'_> {
    fn convert_error(&mut self, err: Error) -> wasmtime::Result<Error> {
        Ok(err)
    }
}
impl HostClient for WasiMessagingCtxView<'_> {}
impl HostMessage for WasiMessagingCtxView<'_> {}

pub fn get_client<T>(
    accessor: &Accessor<T, WasiMessaging>, self_: &Resource<ClientProxy>,
) -> Result<ClientProxy> {
    accessor.with(|mut store| {
        let client = store.get().table.get(self_)?;
        Ok::<_, Error>(client.clone())
    })
}

pub fn get_message<T>(
    accessor: &Accessor<T, WasiMessaging>, self_: &Resource<MessageProxy>,
) -> Result<MessageProxy> {
    accessor.with(|mut store| {
        let message = store.get().table.get(self_)?;
        Ok::<_, Error>(message.clone())
    })
}
