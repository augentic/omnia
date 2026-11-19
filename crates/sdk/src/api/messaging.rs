//! Typed messaging routing over application handlers.
//!
//! A module registers its topics on a [`Handlers`](crate::api::Handlers) of
//! its own and the guest merges each (`Handlers::from([..])` or
//! [`Handlers::merge`](crate::api::Handlers::merge)), closing the whole with
//! [`Handlers::into_router`](crate::api::Handlers::into_router) into its one
//! [`Router`](crate::api::Router). A topic decodes JSON unless registered
//! with [`Handlers::topic_with`](crate::api::Handlers::topic_with).
//! [`export!`] binds the router fn as the guest's `omnia:messaging` handler.

use std::collections::BTreeMap;
use std::fmt;
use std::future::Future;
use std::marker::PhantomData;
use std::pin::Pin;
use std::sync::Arc;
use std::time::SystemTime;

use bytes::Bytes;
use serde::de::DeserializeOwned;
use tracing::Instrument as _;

use crate::api::{Context, Handler, Metadata};

#[doc(hidden)]
#[macro_export]
macro_rules! __export_messaging {
    ($router:path $(,)?) => {
        #[cfg(all(target_arch = "wasm32", target_os = "wasi"))]
        const _: () = {
            struct MessagingGuest;

            $crate::messaging::export!(MessagingGuest);

            impl $crate::exports::messaging::incoming_handler::Guest for MessagingGuest {
                async fn handle(
                    delivery: $crate::messaging::types::Delivery,
                ) -> ::core::result::Result<(), $crate::messaging::types::Settle> {
                    $crate::api::messaging::handle(&$router(), delivery).await
                }
            }
        };
    };
}

/// Bind a router fn as the guest's `omnia:messaging` export.
///
/// Calls the router fn on each delivery and dispatches the delivery through
/// the [`Router`](crate::api::Router) it returns, as `handle` does. Expands
/// to nothing on native targets.
///
/// ```rust,ignore
/// use omnia_sdk::api::messaging;
///
/// messaging::export!(router);
/// ```
#[doc(inline)]
pub use crate::__export_messaging as export;

/// An owned inbound delivery independent of a messaging binding.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
#[non_exhaustive]
pub struct Delivery {
    /// The exact topic supplied by the host.
    pub topic: Option<String>,

    /// The opaque message payload.
    pub payload: Bytes,

    /// The key that orders and partitions the message, when the publisher set
    /// one with [`Message::with_key`](crate::messaging::types::Message::with_key).
    pub key: Option<String>,

    /// The optional payload media type.
    pub content_type: Option<String>,

    /// Transport metadata in delivery order.
    pub metadata: Vec<(String, String)>,

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
/// [`export!`] expands to this call, and a hand-written `omnia:messaging`
/// export calls it directly. What `handle` returns settles the delivery, and
/// a failure carries its [`Error`](crate::Error)'s text as the reason:
///
/// - A [`BadRequest`](crate::Error::BadRequest) rejects the delivery, as a
///   message no redelivery handles.
/// - Any other failure releases it, as a failure that may pass.
///
/// # Errors
///
/// Returns how a delivery the router failed to handle settles.
#[cfg(target_arch = "wasm32")]
#[crate::otel::instrument(name = "messaging_guest_handle")]
pub async fn handle(
    router: &crate::api::Router, delivery: crate::messaging::types::Delivery,
) -> Result<(), crate::messaging::types::Settle> {
    use std::time::{Duration, UNIX_EPOCH};

    use crate::messaging::types::{Release, Settle};

    let crate::messaging::types::Delivery {
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
            crate::Error::BadRequest { .. } => Settle::Reject(reason),
            _ => Settle::Release(Release {
                reason,
                delay_ms: None,
            }),
        }
    })
}

#[derive(Clone, Default)]
pub(crate) struct Topics {
    routes: BTreeMap<String, Arc<dyn ErasedRoute>>,
}

impl Topics {
    pub(crate) fn route<F, I>(self, topic: impl Into<String>, handler: F) -> Self
    where
        F: Handler<I>,
        I: DeserializeOwned + Send + 'static,
    {
        self.route_with(topic, handler, |delivery: &Delivery| {
            serde_json::from_slice(&delivery.payload)
                .map_err(|error| crate::bad_request!("malformed JSON payload: {error}"))
        })
    }

    pub(crate) fn route_with<F, I, D>(self, topic: impl Into<String>, handler: F, decode: D) -> Self
    where
        F: Handler<I>,
        I: Send + 'static,
        D: Fn(&Delivery) -> crate::Result<I> + Send + Sync + 'static,
    {
        let route = Route::<F, I, D> {
            handler,
            decode,
            marker: PhantomData,
        };
        self.insert(topic.into(), Arc::new(route))
    }

    pub(crate) fn merge(self, other: Self) -> Self {
        other.routes.into_iter().fold(self, |routes, (topic, route)| routes.insert(topic, route))
    }

    pub(crate) fn topics(&self) -> impl Iterator<Item = &str> {
        self.routes.keys().map(String::as_str)
    }

    pub(crate) async fn handle(&self, delivery: Delivery) -> crate::Result<()> {
        let topic = delivery.topic.as_deref().ok_or_else(|| {
            crate::Error::bad_request("missing-topic", "the delivery has no topic")
        })?;
        let route = self.routes.get(topic).ok_or_else(|| {
            crate::Error::not_found("unhandled-topic", format!("no route for topic `{topic}`"))
        })?;
        route.dispatch(&delivery).await
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

type DispatchFuture<'a> = Pin<Box<dyn Future<Output = crate::Result<()>> + Send + 'a>>;

trait ErasedRoute: Send + Sync {
    fn dispatch<'a>(&'a self, delivery: &'a Delivery) -> DispatchFuture<'a>;
}

struct Route<F, I, D> {
    handler: F,
    decode: D,
    marker: PhantomData<fn() -> I>,
}

impl<F, I, D> ErasedRoute for Route<F, I, D>
where
    F: Handler<I>,
    I: Send + 'static,
    D: Fn(&Delivery) -> crate::Result<I> + Send + Sync + 'static,
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
                self.handler
                    .clone()
                    .call(Context::new(metadata), input)
                    .await
                    .map(|_| ())
                    .map_err(|error| crate::Error::from_chain(&error))
            }
            .instrument(span),
        )
    }
}
