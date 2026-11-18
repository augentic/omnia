use omnia_core::HostView;
use wasmtime::component::{Accessor, Resource};

use crate::host::generated::omnia::messaging::producer::{Host, HostWithStore};
use crate::host::generated::omnia::messaging::types::{self, Topic};
use crate::host::resource::{ClientProxy, Message};
use crate::host::types_impl::get_client;
use crate::host::{MessagingClient, Result, WasiMessaging};

impl<T> HostWithStore<T> for WasiMessaging {
    async fn send(
        accessor: &Accessor<T, Self>, c: Resource<ClientProxy>, topic: Topic,
        message: types::Message,
    ) -> Result<()> {
        let client = get_client(accessor, &c)?;
        let message = Message::try_from(message)?;
        client.send(topic, message).await?;

        Ok(())
    }
}

impl Host for HostView<'_, dyn MessagingClient> {}
