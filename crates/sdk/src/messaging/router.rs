//! The topic routes a [`Router`](crate::Router) dispatches each inbound
//! delivery through.

use std::collections::BTreeMap;
use std::fmt;
use std::future::Future;
use std::marker::PhantomData;
use std::pin::Pin;
use std::sync::Arc;
use std::time::SystemTime;

use bytes::Bytes;
use serde::Serialize;
use serde::de::DeserializeOwned;
use tracing::Instrument as _;

use crate::messaging::Message;
use crate::{Context, Handler, Metadata};

#[doc(hidden)]
#[macro_export]
macro_rules! __export_messaging {
    ($router:path $(,)?) => {
        #[cfg(all(target_arch = "wasm32", target_os = "wasi"))]
        const _: () = {
            struct MessagingGuest;

            $crate::exports::messaging::export!(MessagingGuest);

            impl $crate::exports::messaging::incoming_handler::Guest for MessagingGuest {
                async fn handle(
                    delivery: $crate::messaging::types::Delivery,
                ) -> ::core::result::Result<(), $crate::messaging::types::Settle> {
                    $crate::messaging::handle(&$router(), delivery).await
                }
            }
        };

        // naming the fn keeps a private one from reading as dead code
        #[cfg(not(all(target_arch = "wasm32", target_os = "wasi")))]
        const _: () = {
            let _ = $router;
        };
    };
}

/// An owned inbound delivery independent of a messaging binding.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
#[non_exhaustive]
pub struct Delivery {
    /// The exact topic supplied by the host.
    pub topic: Option<String>,

    /// The opaque message payload.
    pub payload: Bytes,

    /// The key that orders and partitions the message, when the publisher set
    /// one with [`Message::with_key`](crate::messaging::Message::with_key).
    pub key: Option<String>,

    /// The optional payload media type.
    pub content_type: Option<String>,

    /// Transport metadata in delivery order.
    pub metadata: Vec<(String, String)>,

    /// The address [`Client::reply`](crate::messaging::Client::reply)
    /// answers, `None` when the sender awaits no reply.
    pub reply_to: Option<String>,

    /// Where the broker stored the message, `None` where it stores none.
    ///
    /// A redelivery carries the same position.
    pub position: Option<String>,

    /// How many times the broker has delivered the message, counting this
    /// one. `None` where the broker keeps no count.
    pub attempt: Option<u32>,

    /// When the broker took the message, `None` where it records no time.
    pub published_at: Option<SystemTime>,

    /// When the host stops waiting for the handler, which the handler reads
    /// as [`Metadata::deadline`].
    pub deadline: Option<SystemTime>,
}

impl Delivery {
    /// A delivery of `payload` on `topic`, with no key, content type, or metadata.
    #[must_use]
    pub fn new(topic: impl Into<String>, payload: &[u8]) -> Self {
        Self {
            topic: Some(topic.into()),
            payload: Bytes::copy_from_slice(payload),
            ..Self::default()
        }
    }

    /// Set the key that orders and partitions the message.
    #[must_use]
    pub fn with_key(mut self, key: impl Into<String>) -> Self {
        self.key = Some(key.into());
        self
    }

    /// Set the payload's media type.
    #[must_use]
    pub fn with_content_type(mut self, content_type: impl Into<String>) -> Self {
        self.content_type = Some(content_type.into());
        self
    }

    /// Append a transport metadata entry.
    #[must_use]
    pub fn with_metadata(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.metadata.push((name.into(), value.into()));
        self
    }

    /// Set the address a reply answers.
    #[must_use]
    pub fn with_reply_to(mut self, reply_to: impl Into<String>) -> Self {
        self.reply_to = Some(reply_to.into());
        self
    }

    /// Set where the broker stored the message.
    #[must_use]
    pub fn with_position(mut self, position: impl Into<String>) -> Self {
        self.position = Some(position.into());
        self
    }

    /// Set how many times the broker has delivered the message.
    #[must_use]
    pub const fn with_attempt(mut self, attempt: u32) -> Self {
        self.attempt = Some(attempt);
        self
    }

    /// Set when the broker took the message.
    #[must_use]
    pub const fn with_published_at(mut self, published_at: SystemTime) -> Self {
        self.published_at = Some(published_at);
        self
    }

    /// Set when the host stops waiting for the handler.
    #[must_use]
    pub const fn with_deadline(mut self, deadline: SystemTime) -> Self {
        self.deadline = Some(deadline);
        self
    }
}

/// Dispatch one `omnia:messaging` delivery through `router`.
///
/// The delivery runs inside the guest telemetry scope.
/// [`export!`](crate::messaging::export!) expands to this call, and a
/// hand-written `omnia:messaging` export calls it directly.
///
/// A [`request`](crate::Handlers::request) route's reply goes to the
/// delivery's [`reply_to`](Delivery::reply_to) through the
/// [`DEFAULT`](crate::messaging::DEFAULT) broker, and a delivery that awaits
/// no reply drops it.
///
/// What `handle` returns settles the delivery, and a failure carries its
/// [`Error`](crate::Error)'s text as the reason:
///
/// - A [`BadRequest`](crate::ErrorKind::BadRequest) or a
///   [`Conflict`](crate::ErrorKind::Conflict) rejects the delivery, as a
///   message no redelivery handles.
/// - Any other failure releases it, as a failure that may pass, and leaves
///   the delay to the backend.
///
/// A reply the broker does not take fails the delivery the same way.
///
/// A guest that sets a release delay implements
/// [`exports::messaging`](crate::exports::messaging) instead.
///
/// # Errors
///
/// Returns how a delivery the router failed to handle settles.
#[crate::otel::instrument(name = "messaging_guest_handle")]
pub async fn handle(
    router: &crate::Router, delivery: crate::messaging::types::Delivery,
) -> Result<(), crate::messaging::types::Settle> {
    use std::time::{Duration, UNIX_EPOCH};

    use crate::ErrorKind;
    use crate::messaging::types::{Release, Settle};

    let instant = |millis| UNIX_EPOCH.checked_add(Duration::from_millis(millis));
    let crate::messaging::types::Delivery {
        topic,
        message,
        reply_to,
        position,
        attempt,
        published_at,
        deadline,
    } = delivery;
    let delivery = Delivery {
        topic: (!topic.is_empty()).then_some(topic),
        payload: Bytes::from(message.data),
        key: message.key,
        content_type: message.content_type,
        metadata: message.metadata,
        reply_to,
        position,
        attempt,
        published_at: published_at.and_then(instant),
        deadline: deadline.and_then(instant),
    };
    let settle = |error: crate::Error| {
        let reason = error.to_string();
        match error.kind() {
            ErrorKind::BadRequest | ErrorKind::Conflict => Settle::Reject(reason),
            _ => Settle::Release(Release {
                reason,
                delay_ms: None,
            }),
        }
    };
    let reply_to = delivery.reply_to.clone();
    let reply = router.handle(delivery).await.map_err(settle)?;

    if let (Some(reply), Some(reply_to)) = (reply, reply_to) {
        let client = crate::messaging::default().await.map_err(settle)?;
        client.reply(&reply_to, reply).await.map_err(settle)?;
    }
    Ok(())
}

#[derive(Clone, Default)]
pub struct Topics {
    routes: BTreeMap<String, Arc<dyn ErasedRoute>>,
}

impl Topics {
    pub fn route<F, I, M>(self, topic: impl Into<String>, handler: F) -> Self
    where
        F: Handler<I, M>,
        I: DeserializeOwned + Send + 'static,
        M: 'static,
    {
        self.route_with(topic, handler, json)
    }

    pub fn route_with<F, I, M, D>(self, topic: impl Into<String>, handler: F, decode: D) -> Self
    where
        F: Handler<I, M>,
        I: Send + 'static,
        M: 'static,
        D: Fn(&Delivery) -> crate::Result<I> + Send + Sync + 'static,
    {
        self.register(topic, handler, decode, |_output| Ok(None))
    }

    pub fn request<F, I, M>(self, topic: impl Into<String>, handler: F) -> Self
    where
        F: Handler<I, M>,
        F::Output: Serialize,
        I: DeserializeOwned + Send + 'static,
        M: 'static,
    {
        self.register(topic, handler, json, |output| Message::json(&output).map(Some))
    }

    pub fn request_with<F, I, M, D>(self, topic: impl Into<String>, handler: F, decode: D) -> Self
    where
        F: Handler<I, M, Output = Message>,
        I: Send + 'static,
        M: 'static,
        D: Fn(&Delivery) -> crate::Result<I> + Send + Sync + 'static,
    {
        self.register(topic, handler, decode, |reply| Ok(Some(reply)))
    }

    pub fn merge(self, other: Self) -> Self {
        other.routes.into_iter().fold(self, |routes, (topic, route)| routes.insert(topic, route))
    }

    pub fn topics(&self) -> impl Iterator<Item = &str> {
        self.routes.keys().map(String::as_str)
    }

    pub async fn handle(&self, delivery: Delivery) -> crate::Result<Option<Message>> {
        let topic = delivery.topic.as_deref().ok_or_else(|| {
            crate::Error::bad_request("missing-topic", "the delivery has no topic")
        })?;
        let route = self.routes.get(topic).ok_or_else(|| {
            crate::Error::not_found("unhandled-topic", format!("no route for topic `{topic}`"))
        })?;
        route.dispatch(&delivery).await
    }

    fn register<F, I, M, D, R>(
        self, topic: impl Into<String>, handler: F, decode: D, reply: R,
    ) -> Self
    where
        F: Handler<I, M>,
        I: Send + 'static,
        M: 'static,
        D: Fn(&Delivery) -> crate::Result<I> + Send + Sync + 'static,
        R: Fn(F::Output) -> crate::Result<Option<Message>> + Send + Sync + 'static,
    {
        let route = Route::<F, I, M, D, R> {
            handler,
            decode,
            reply,
            marker: PhantomData,
        };
        self.insert(topic.into(), Arc::new(route))
    }

    fn insert(mut self, topic: String, route: Arc<dyn ErasedRoute>) -> Self {
        assert!(!topic.is_empty(), "messaging topic cannot be empty");
        assert!(!self.routes.contains_key(&topic), "duplicate messaging topic `{topic}`");
        self.routes.insert(topic, route);
        self
    }
}

impl fmt::Debug for Topics {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_set().entries(self.routes.keys()).finish()
    }
}

fn json<I: DeserializeOwned>(delivery: &Delivery) -> crate::Result<I> {
    serde_json::from_slice(&delivery.payload)
        .map_err(|error| crate::bad_request!("malformed JSON payload: {error}"))
}

type DispatchFuture<'a> = Pin<Box<dyn Future<Output = crate::Result<Option<Message>>> + Send + 'a>>;

trait ErasedRoute: Send + Sync {
    fn dispatch<'a>(&'a self, delivery: &'a Delivery) -> DispatchFuture<'a>;
}

struct Route<F, I, M, D, R> {
    handler: F,
    decode: D,
    reply: R,
    marker: PhantomData<fn() -> (I, M)>,
}

impl<F, I, M, D, R> ErasedRoute for Route<F, I, M, D, R>
where
    F: Handler<I, M>,
    I: Send + 'static,
    M: 'static,
    D: Fn(&Delivery) -> crate::Result<I> + Send + Sync + 'static,
    R: Fn(F::Output) -> crate::Result<Option<Message>> + Send + Sync + 'static,
{
    fn dispatch<'a>(&'a self, delivery: &'a Delivery) -> DispatchFuture<'a> {
        let metadata = Metadata {
            deadline: delivery.deadline,
            ..Metadata::from_lookup(|name| {
                delivery
                    .metadata
                    .iter()
                    .find(|(key, _)| key.eq_ignore_ascii_case(name))
                    .map(|(_, value)| value.clone())
            })
        };
        let span = tracing::info_span!(
            "handler",
            request_id = metadata.request_id.as_deref(),
            correlation_id = metadata.correlation_id.as_deref(),
            topic = delivery.topic.as_deref(),
        );
        Box::pin(
            async move {
                let input = (self.decode)(delivery).map_err(|error| {
                    crate::Error::bad_request("undecodable", error.description())
                })?;
                let output = self
                    .handler
                    .clone()
                    .call(Context::new(metadata), input)
                    .await
                    .map_err(|error| crate::Error::from_chain(&error))?;
                (self.reply)(output)
            }
            .instrument(span),
        )
    }
}
