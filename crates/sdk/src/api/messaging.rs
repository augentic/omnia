//! Typed messaging routing over application handlers.
//!
//! A module registers its topics on a [`Handlers`](crate::api::Handlers) of
//! its own and the guest merges each (`Handlers::from([..])` or
//! [`Handlers::merge`](crate::api::Handlers::merge)) into its one
//! [`Router`](crate::api::Router) over a [`Client`], closing the whole with
//! [`Handlers::with_state`](crate::api::Handlers::with_state). A topic
//! decodes JSON unless registered with
//! [`Handlers::topic_with`](crate::api::Handlers::topic_with).
//! [`export!`] binds the router fn as the guest's `wasi:messaging` handler.

use std::collections::BTreeMap;
use std::error::Error;
use std::future::Future;
use std::marker::PhantomData;
use std::pin::Pin;
use std::sync::Arc;
use std::{fmt, iter};

use bytes::Bytes;
use serde::de::DeserializeOwned;
use tracing::Instrument as _;

use crate::api::{Client, DecodeError, Handler, Metadata};
use crate::capability::{self, ErrorKind};

#[doc(hidden)]
#[macro_export]
macro_rules! __export_messaging {
    ($router:path $(,)?) => {
        $crate::api::messaging::export!($router, $crate::WasiProvider);
    };
    ($router:path, $provider:expr $(,)?) => {
        #[cfg(all(target_arch = "wasm32", target_os = "wasi"))]
        const _: () = {
            struct MessagingGuest;

            $crate::omnia_wasi_messaging::export!(
                MessagingGuest with_types_in $crate::omnia_wasi_messaging
            );

            impl $crate::omnia_wasi_messaging::incoming_handler::Guest for MessagingGuest {
                async fn handle(
                    message: $crate::omnia_wasi_messaging::types::Message,
                ) -> ::core::result::Result<(), $crate::omnia_wasi_messaging::types::Error>
                {
                    $crate::api::messaging::handle_wasi($router, $provider, message).await
                }
            }
        };
    };
}

/// Bind a router fn as the guest's `wasi:messaging` export.
///
/// Calls the router fn on each delivery with `WasiProvider`, or with the
/// provider expression passed as a second argument, and dispatches the
/// delivery through the resulting [`Router`](crate::api::Router). Expands to
/// nothing on native targets, where a test or a native host serves the router
/// itself.
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
    /// one with [`Message::with_key`](crate::messaging::Message::with_key).
    pub key: Option<String>,

    /// The optional payload media type.
    pub content_type: Option<String>,

    /// Transport metadata in delivery order.
    pub metadata: Vec<(String, String)>,
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
}

/// Why a [`Router`](crate::api::Router) did not handle a delivery.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DeliveryError {
    /// The delivery had no topic.
    MissingTopic,
    /// No route was registered for the exact topic.
    UnhandledTopic(String),
    /// The payload did not decode into the route's input.
    Undecodable(String),
    /// The handler returned an error.
    Failed {
        /// The class of the handler's error, from the first match in its source chain.
        ///
        /// - An [`Error`](crate::Error) maps by variant:
        ///   [`BadRequest`](crate::Error::BadRequest) is
        ///   [`Invalid`](ErrorKind::Invalid), [`NotFound`](crate::Error::NotFound)
        ///   is [`NotFound`](ErrorKind::NotFound),
        ///   [`ServerError`](crate::Error::ServerError) is
        ///   [`Other`](ErrorKind::Other), and
        ///   [`BadGateway`](crate::Error::BadGateway) is
        ///   [`Unavailable`](ErrorKind::Unavailable).
        /// - A [`vcs::Error`](crate::vcs::Error), [`plugins::Error`](crate::plugins::Error),
        ///   or [`model::Error`](crate::model::Error) maps as the
        ///   [`Error`](crate::Error) it converts into.
        /// - A [`capability::Error`] keeps its [`kind`](capability::Error::kind).
        /// - Any other error is [`Other`](ErrorKind::Other).
        kind: ErrorKind,
        /// The handler's error, as it displays.
        message: String,
    },
}

impl fmt::Display for DeliveryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingTopic => f.write_str("message is missing topic"),
            Self::UnhandledTopic(topic) => write!(f, "unhandled topic: {topic}"),
            Self::Undecodable(message) | Self::Failed { message, .. } => f.write_str(message),
        }
    }
}

impl Error for DeliveryError {}

/// Adapt a WIT message to an owned delivery and dispatch it.
///
/// The current WIT contract carries only `result<_, error>`. Success
/// acknowledges the delivery, and every [`DeliveryError`] crosses as
/// `error.other` carrying its text.
///
/// # Errors
///
/// Returns the WIT delivery failure.
#[cfg(all(target_arch = "wasm32", target_os = "wasi"))]
pub async fn handle<P: Send + Sync + 'static>(
    router: &crate::api::Router<P>, message: omnia_wasi_messaging::types::Message,
) -> Result<(), omnia_wasi_messaging::types::Error> {
    let metadata = message.metadata().unwrap_or_default();
    let key = metadata
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case(crate::wasi::messaging::KEY))
        .map(|(_, value)| value.clone());
    let delivery = Delivery {
        topic: message.topic(),
        payload: Bytes::from(message.data()),
        key,
        content_type: message.content_type(),
        metadata,
    };
    router
        .handle(delivery)
        .await
        .map_err(|error| omnia_wasi_messaging::types::Error::Other(error.to_string()))
}

/// Dispatch one WIT message through the router `router` builds over
/// `provider`, inside the guest telemetry scope.
///
/// # Errors
///
/// Returns the WIT delivery failure [`handle`] reports.
#[cfg(all(target_arch = "wasm32", target_os = "wasi"))]
#[doc(hidden)]
#[omnia_wasi_otel::instrument(name = "messaging_guest_handle")]
pub async fn handle_wasi<P: Send + Sync + 'static>(
    router: impl FnOnce(P) -> crate::api::Router<P>, provider: P,
    message: omnia_wasi_messaging::types::Message,
) -> Result<(), omnia_wasi_messaging::types::Error> {
    handle(&router(provider), message).await
}

pub(crate) struct Topics<P: Send + Sync + 'static> {
    routes: BTreeMap<String, Arc<dyn ErasedRoute<P>>>,
}

impl<P: Send + Sync + 'static> Topics<P> {
    pub(crate) fn new() -> Self {
        Self {
            routes: BTreeMap::new(),
        }
    }

    pub(crate) fn route<F, I>(self, topic: impl Into<String>, handler: F) -> Self
    where
        F: Handler<P, I>,
        I: DeserializeOwned + Send + 'static,
    {
        self.route_with(topic, handler, |delivery: &Delivery| {
            serde_json::from_slice(&delivery.payload)
                .map_err(|error| DecodeError::new(format!("malformed JSON payload: {error}")))
        })
    }

    pub(crate) fn route_with<F, I, D>(self, topic: impl Into<String>, handler: F, decode: D) -> Self
    where
        F: Handler<P, I>,
        I: Send + 'static,
        D: Fn(&Delivery) -> Result<I, DecodeError> + Send + Sync + 'static,
    {
        let route = Route::<P, F, I, D> {
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

    pub(crate) async fn handle(
        &self, delivery: Delivery, client: &Client<P>,
    ) -> Result<(), DeliveryError> {
        let topic = delivery.topic.as_deref().ok_or(DeliveryError::MissingTopic)?;
        let route = self
            .routes
            .get(topic)
            .ok_or_else(|| DeliveryError::UnhandledTopic(topic.to_owned()))?;
        route.dispatch(&delivery, client).await
    }

    fn insert(mut self, topic: String, route: Arc<dyn ErasedRoute<P>>) -> Self {
        assert!(!topic.is_empty(), "messaging topic cannot be empty");
        assert!(!self.routes.contains_key(&topic), "duplicate messaging topic `{topic}`");
        self.routes.insert(topic, route);
        self
    }
}

impl<P: Send + Sync + 'static> Clone for Topics<P> {
    fn clone(&self) -> Self {
        Self {
            routes: self.routes.clone(),
        }
    }
}

impl<P: Send + Sync + 'static> fmt::Debug for Topics<P> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_set().entries(self.routes.keys()).finish()
    }
}

type DispatchFuture<'a> = Pin<Box<dyn Future<Output = Result<(), DeliveryError>> + Send + 'a>>;

trait ErasedRoute<P: Send + Sync + 'static>: Send + Sync {
    fn dispatch<'a>(&'a self, delivery: &'a Delivery, client: &'a Client<P>) -> DispatchFuture<'a>;
}

struct Route<P, F, I, D> {
    handler: F,
    decode: D,
    marker: PhantomData<fn(P) -> I>,
}

impl<P, F, I, D> ErasedRoute<P> for Route<P, F, I, D>
where
    P: Send + Sync + 'static,
    F: Handler<P, I>,
    I: Send + 'static,
    D: Fn(&Delivery) -> Result<I, DecodeError> + Send + Sync + 'static,
{
    fn dispatch<'a>(&'a self, delivery: &'a Delivery, client: &'a Client<P>) -> DispatchFuture<'a> {
        let metadata = Metadata::from_lookup(|name| {
            delivery
                .metadata
                .iter()
                .find(|(key, _)| key.eq_ignore_ascii_case(name))
                .map(|(_, value)| value.clone())
        });
        let span = tracing::info_span!(
            "handler",
            request_id = metadata.request_id.as_deref(),
            correlation_id = metadata.correlation_id.as_deref(),
            topic = delivery.topic.as_deref(),
        );
        Box::pin(
            async move {
                let input = (self.decode)(delivery)
                    .map_err(|error| DeliveryError::Undecodable(error.to_string()))?;
                client.call(self.handler.clone(), input, &metadata).await.map(|_| ()).map_err(
                    |error| DeliveryError::Failed {
                        kind: error_kind(&error),
                        message: error.to_string(),
                    },
                )
            }
            .instrument(span),
        )
    }
}

fn error_kind(error: &(dyn Error + 'static)) -> ErrorKind {
    iter::successors(Some(error), |&error| error.source())
        .find_map(|error| {
            error
                .downcast_ref::<capability::Error>()
                .map(capability::Error::kind)
                .or_else(|| crate::Error::from_domain(error).as_ref().map(crate::Error::kind))
        })
        .unwrap_or(ErrorKind::Other)
}
