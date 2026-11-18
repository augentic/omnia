//! The `omnia:messaging/request-reply` host bindings.

use std::time::Duration;

use omnia_core::HostView;
use wasmtime::component::{Accessor, Resource};

use crate::host::generated::omnia::messaging::request_reply::{Host, HostWithStore};
use crate::host::generated::omnia::messaging::types::{self, Topic};
use crate::host::resource::{ClientProxy, Message, RequestOptions};
use crate::host::types_impl::get_client;
use crate::host::{Error, MessagingClient, Result, WasiMessaging};

impl<T> HostWithStore<T> for WasiMessaging {
    async fn request(
        accessor: &Accessor<T, Self>, c: Resource<ClientProxy>, topic: Topic,
        message: types::Message, timeout_ms: Option<u32>,
    ) -> Result<types::Message> {
        if timeout_ms == Some(0) {
            return Err(Error::Invalid("a request waits at least 1 ms".to_owned()));
        }
        let client = get_client(accessor, &c)?;
        let request = Message::try_from(message)?;
        let options = timeout_ms.map(|timeout_ms| RequestOptions {
            timeout: Some(Duration::from_millis(u64::from(timeout_ms))),
            ..RequestOptions::default()
        });

        let reply = client.request(topic, request, options).await?;
        Ok(reply.into())
    }

    async fn reply(
        accessor: &Accessor<T, Self>, reply_to: String, message: types::Message,
    ) -> Result<()> {
        if reply_to.is_empty() {
            return Err(Error::Invalid("a reply needs the address a delivery carried".to_owned()));
        }
        let message = Message::try_from(message)?;

        let client = accessor.with(|mut store| store.get().client.endpoint()).await?;
        client.send(reply_to, message).await?;

        Ok(())
    }
}

impl Host for HostView<'_, dyn MessagingClient> {}
