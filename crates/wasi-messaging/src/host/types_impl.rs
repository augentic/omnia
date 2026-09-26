use wasmtime::component::{Access, Accessor, Resource};

use crate::host::generated::wasi::messaging::types;
pub use crate::host::generated::wasi::messaging::types::{
    Error, Host, HostClient, HostClientWithStore, HostMessage, HostMessageWithStore, Topic,
};
use crate::host::resource::{ClientProxy, Message};
use crate::host::{Result, WasiMessaging, WasiMessagingCtxView};

impl<T> HostClientWithStore<T> for WasiMessaging {
    async fn connect(accessor: &Accessor<T, Self>, _name: String) -> Result<Resource<ClientProxy>> {
        let client = accessor.with(|mut store| store.get().ctx.connect()).await?;
        let proxy = omnia_core::Proxy(client);
        Ok(accessor.with(|mut store| store.get().table.push(proxy))?)
    }

    fn disconnect(_: Access<'_, T, Self>, _: Resource<ClientProxy>) -> Result<()> {
        Ok(())
    }

    fn drop(mut accessor: Access<'_, T, Self>, rep: Resource<ClientProxy>) -> wasmtime::Result<()> {
        Ok(accessor.get().table.delete(rep).map(|_| ())?)
    }
}

impl<T> HostMessageWithStore<T> for WasiMessaging {
    fn new(mut host: Access<'_, T, Self>, data: Vec<u8>) -> wasmtime::Result<Resource<Message>> {
        Ok(host.get().table.push(Message::new(data))?)
    }

    fn topic(
        mut host: Access<'_, T, Self>, self_: Resource<Message>,
    ) -> wasmtime::Result<Option<Topic>> {
        let message = host.get().table.get(&self_)?;
        if message.topic.is_empty() { Ok(None) } else { Ok(Some(message.topic.clone())) }
    }

    fn content_type(
        mut host: Access<'_, T, Self>, self_: Resource<Message>,
    ) -> wasmtime::Result<Option<String>> {
        let message = host.get().table.get(&self_)?;
        Ok(message.metadata.as_ref().and_then(|md| md.get("content-type").cloned()))
    }

    fn set_content_type(
        mut host: Access<'_, T, Self>, self_: Resource<Message>, content_type: String,
    ) -> wasmtime::Result<()> {
        let message = host.get().table.get_mut(&self_)?;
        message.metadata.get_or_insert_default().insert("content-type".to_string(), content_type);
        Ok(())
    }

    fn data(mut host: Access<'_, T, Self>, self_: Resource<Message>) -> wasmtime::Result<Vec<u8>> {
        let message = host.get().table.get(&self_)?;
        Ok(message.payload.clone())
    }

    fn set_data(
        mut host: Access<'_, T, Self>, self_: Resource<Message>, data: Vec<u8>,
    ) -> wasmtime::Result<()> {
        host.get().table.get_mut(&self_)?.payload = data;
        Ok(())
    }

    fn metadata(
        mut host: Access<'_, T, Self>, self_: Resource<Message>,
    ) -> wasmtime::Result<Option<types::Metadata>> {
        let message = host.get().table.get(&self_)?;
        Ok(message.metadata.clone().map(Into::into))
    }

    fn add_metadata(
        mut host: Access<'_, T, Self>, self_: Resource<Message>, key: String, value: String,
    ) -> wasmtime::Result<()> {
        let message = host.get().table.get_mut(&self_)?;
        message.metadata.get_or_insert_default().insert(key, value);
        Ok(())
    }

    fn set_metadata(
        mut host: Access<'_, T, Self>, self_: Resource<Message>, meta: types::Metadata,
    ) -> wasmtime::Result<()> {
        host.get().table.get_mut(&self_)?.metadata = Some(meta.into());
        Ok(())
    }

    fn remove_metadata(
        mut host: Access<'_, T, Self>, self_: Resource<Message>, key: String,
    ) -> wasmtime::Result<()> {
        let message = host.get().table.get_mut(&self_)?;
        if let Some(md) = message.metadata.as_mut() {
            md.remove(&key);
        }
        Ok(())
    }

    fn drop(mut accessor: Access<'_, T, Self>, rep: Resource<Message>) -> wasmtime::Result<()> {
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
    Ok(omnia_core::get_cloned(accessor, self_)?)
}

pub fn get_message<T>(
    accessor: &Accessor<T, WasiMessaging>, self_: &Resource<Message>,
) -> Result<Message> {
    Ok(omnia_core::get_cloned(accessor, self_)?)
}
