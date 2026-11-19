//! [`Publisher`] over `omnia:messaging`, and the guest's `omnia:messaging` export.

use std::time::{Duration, UNIX_EPOCH};

use bytes::Bytes;
use omnia_wasi_messaging::omnia::producer;
use omnia_wasi_messaging::omnia::types::{self as wire, Client};

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

/// Dispatch one `omnia:messaging` delivery through `router`.
///
/// The delivery runs inside the guest telemetry scope.
/// [`messaging::export!`](crate::api::messaging::export!) expands to this
/// call, and a hand-written `omnia:messaging` export calls it directly.
///
/// What `handle` returns settles the delivery. A failure carries its
/// [`Error`]'s text as the reason:
///
/// - A [`BadRequest`](Error::BadRequest) rejects the delivery, as a message no
///   redelivery handles.
/// - Any other failure releases it, as a failure that may pass.
///
/// # Errors
///
/// Returns how a delivery the router failed to handle settles.
#[omnia_wasi_otel::instrument(name = "messaging_guest_handle")]
pub async fn handle<P: Send + Sync + 'static>(
    router: &Router<P>, delivery: wire::Delivery,
) -> Result<(), wire::Settle> {
    let wire::Delivery {
        topic,
        message,
        deadline,
        ..
    } = delivery;
    let delivery = Delivery {
        topic: (!topic.is_empty()).then_some(topic),
        payload: Bytes::from(message.data),
        key: message.key,
        content_type: message.content_type,
        metadata: message.metadata,
        deadline: deadline.and_then(|millis| UNIX_EPOCH.checked_add(Duration::from_millis(millis))),
    };
    router.handle(delivery).await.map_err(|error| {
        let reason = error.to_string();
        match error {
            Error::BadRequest { .. } => wire::Settle::Reject(reason),
            _ => wire::Settle::Release(wire::Release {
                reason,
                delay_ms: None,
            }),
        }
    })
}
