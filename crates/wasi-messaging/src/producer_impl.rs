//! The `omnia:messaging/producer` host bindings.

use omnia_core::HostView;
use wasmtime::component::{Accessor, Resource};

use crate::generated::omnia::messaging::producer::{Host, HostWithStore};
use crate::generated::omnia::messaging::types::{self, Topic};
use crate::resource::{ClientProxy, Message};
use crate::types_impl::get_client;
use crate::{MessagingClient, Result, WasiMessaging};

impl<T> HostWithStore<T> for WasiMessaging {
    async fn send(
        accessor: &Accessor<T, Self>, client: Resource<ClientProxy>, topic: Topic,
        message: types::Message,
    ) -> Result<()> {
        let client = get_client(accessor, &client)?;
        let message = Message::from(message);
        client.send(topic, message).await?;

        Ok(())
    }
}

impl Host for HostView<'_, dyn MessagingClient> {}
