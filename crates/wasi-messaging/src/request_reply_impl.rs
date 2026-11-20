//! The `omnia:messaging/request-reply` host bindings.

use std::time::Duration;

use omnia_core::HostView;
use wasmtime::component::{Accessor, Resource};

use crate::generated::omnia::messaging::request_reply::{Host, HostWithStore};
use crate::generated::omnia::messaging::types::{self, Topic};
use crate::resource::{ClientProxy, Message};
use crate::types_impl::get_client;
use crate::{Error, MessagingClient, Result, WasiMessaging};

impl<T> HostWithStore<T> for WasiMessaging {
    async fn request(
        accessor: &Accessor<T, Self>, client: Resource<ClientProxy>, topic: Topic,
        message: types::Message, timeout_ms: Option<u64>,
    ) -> Result<types::Message> {
        if timeout_ms == Some(0) {
            return Err(Error::Invalid("a request waits at least 1 ms".to_owned()));
        }
        let client = get_client(accessor, &client)?;
        let request = Message::from(message);
        let timeout = timeout_ms.map(Duration::from_millis);

        let reply = client.request(topic, request, timeout).await?;
        Ok(reply.into())
    }

    async fn reply(
        accessor: &Accessor<T, Self>, client: Resource<ClientProxy>, reply_to: String,
        message: types::Message,
    ) -> Result<()> {
        if reply_to.is_empty() {
            return Err(Error::Invalid("a reply needs the address a delivery carried".to_owned()));
        }
        let client = get_client(accessor, &client)?;
        let message = Message::from(message);
        client.send(reply_to, message).await?;

        Ok(())
    }
}

impl Host for HostView<'_, dyn MessagingClient> {}
