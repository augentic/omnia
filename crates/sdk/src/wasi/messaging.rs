//! [`Publisher`] over `omnia:messaging`, and the guest's `omnia:messaging` export.

use bytes::Bytes;
use omnia_wasi_messaging::producer;
use omnia_wasi_messaging::types::{self as wire, Client};

use crate::api::Router;
use crate::api::messaging::Delivery;
use crate::messaging::Message;
use crate::{Error, Publisher, Result, WasiProvider};

// the broker an unnamed handle connects to
const DEFAULT: &str = "default";

impl Publisher for WasiProvider {
    async fn publish(&self, topic: &str, message: &Message) -> Result<()> {
        let client = Client::connect(DEFAULT.to_owned()).await?;
        let outgoing = wire::Message {
            data: message.payload.to_vec(),
            content_type: message.content_type.clone(),
            id: None,
            key: message.key.clone(),
            metadata: message
                .headers
                .iter()
                .map(|(name, value)| (name.clone(), value.clone()))
                .collect(),
        };
        Ok(producer::send(&client, topic.to_owned(), outgoing).await?)
    }
}

impl From<wire::Error> for Error {
    fn from(err: wire::Error) -> Self {
        match err {
            wire::Error::NotFound(message) => Self::server_error("not-found", message),
            wire::Error::Denied(message) => Self::server_error("denied", message),
            wire::Error::Invalid(message) => Self::server_error("invalid", message),
            wire::Error::Unsupported(message) => Self::server_error("unsupported", message),
            wire::Error::Unavailable(message) => Self::bad_gateway("unavailable", message),
            wire::Error::Other(message) => Self::server_error("other", message),
        }
    }
}

/// Dispatch one `omnia:messaging` delivery through `router`.
///
/// The delivery runs inside the guest telemetry scope.
/// [`messaging::export!`](crate::api::messaging::export!) expands to this
/// call, and a hand-written `omnia:messaging` export calls it directly.
///
/// What `handle` returns settles the delivery. A failure crosses carrying its
/// [`Error`]'s text, as the case its class settles as:
///
/// - A [`BadRequest`](Error::BadRequest) is `invalid`, which no redelivery handles.
/// - A [`BadGateway`](Error::BadGateway) is `unavailable`, which may pass.
/// - Any other failure is `other`, which the backend settles as it decides.
///
/// # Errors
///
/// Returns the WIT delivery failure.
#[omnia_wasi_otel::instrument(name = "messaging_guest_handle")]
pub async fn handle<P: Send + Sync + 'static>(
    router: &Router<P>, delivery: wire::Delivery,
) -> Result<(), wire::Error> {
    let wire::Delivery { topic, message, .. } = delivery;
    let delivery = Delivery {
        topic: (!topic.is_empty()).then_some(topic),
        payload: Bytes::from(message.data),
        key: message.key,
        content_type: message.content_type,
        metadata: message.metadata,
    };
    router.handle(delivery).await.map_err(|error| {
        let text = error.to_string();
        match error {
            Error::BadRequest { .. } => wire::Error::Invalid(text),
            Error::BadGateway { .. } => wire::Error::Unavailable(text),
            _ => wire::Error::Other(text),
        }
    })
}
