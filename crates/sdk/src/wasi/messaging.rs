//! [`Publisher`] over `wasi:messaging`, and the guest's `wasi:messaging` export.

use bytes::Bytes;
use omnia_wasi_messaging::producer;
use omnia_wasi_messaging::types::{self as wire, Client};

use crate::api::Router;
use crate::api::messaging::Delivery;
use crate::messaging::{KEY, Message};
use crate::{Error, Publisher, Result, WasiProvider};

impl Publisher for WasiProvider {
    async fn publish(&self, topic: &str, message: &Message) -> Result<()> {
        let client = Client::connect("host".to_owned()).await?;
        let outgoing = wire::Message::new(&message.payload);
        if let Some(content_type) = &message.content_type {
            outgoing.set_content_type(content_type);
        }
        for (name, value) in &message.headers {
            outgoing.add_metadata(name, value);
        }
        if let Some(key) = &message.key {
            outgoing.add_metadata(KEY, key);
        }
        Ok(producer::send(&client, topic.to_owned(), outgoing).await?)
    }
}

impl From<wire::Error> for Error {
    fn from(err: wire::Error) -> Self {
        match err {
            wire::Error::Timeout => Self::bad_gateway("unavailable", "the broker timed out"),
            wire::Error::Connection(message) => Self::bad_gateway("unavailable", message),
            wire::Error::PermissionDenied(message) => Self::server_error("denied", message),
            wire::Error::Other(message) => Self::server_error("other", message),
        }
    }
}

/// Dispatch one `wasi:messaging` delivery through `router`.
///
/// The delivery runs inside the guest telemetry scope.
/// [`messaging::export!`](crate::api::messaging::export!) expands to this
/// call, and a hand-written `wasi:messaging` export calls it directly.
///
/// The current WIT contract carries only `result<_, error>`. Success
/// acknowledges the delivery, and every failure crosses as `error.other`
/// carrying its [`Error`]'s text.
///
/// # Errors
///
/// Returns the WIT delivery failure.
#[omnia_wasi_otel::instrument(name = "messaging_guest_handle")]
pub async fn handle<P: Send + Sync + 'static>(
    router: &Router<P>, message: wire::Message,
) -> Result<(), wire::Error> {
    let metadata = message.metadata().unwrap_or_default();
    let key = metadata
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case(KEY))
        .map(|(_, value)| value.clone());
    let delivery = Delivery {
        topic: message.topic(),
        payload: Bytes::from(message.data()),
        key,
        content_type: message.content_type(),
        metadata,
    };
    router.handle(delivery).await.map_err(|error| wire::Error::Other(error.to_string()))
}
