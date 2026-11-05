//! Typed messaging routing over application handlers.
//!
//! A module registers its topics on a [`Routes`] of its own and the guest
//! merges each (`Routes::from([..])` or [`Routes::merge`]) into its one
//! [`Router`] over a [`Client`], closing the whole with
//! [`Routes::with_state`] as [`http::Routes`](crate::api::http::Routes)
//! does. A route decodes JSON unless registered with
//! [`Routes::route_with`], and [`Routes::map`] re-keys a module's topics
//! for the deployment, a prefix say, as they are merged. [`export!`] binds
//! the router fn as the guest's `wasi:messaging` handler.

use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;
use std::future::Future;
use std::marker::PhantomData;
use std::pin::Pin;
use std::sync::Arc;

use serde::de::DeserializeOwned;

use crate::api::{Client, DecodeError, Handler, Metadata};

#[doc(hidden)]
#[macro_export]
macro_rules! __export_messaging {
    ($router:path $(,)?) => {
        $crate::api::messaging::export!($router, $crate::WasiProvider);
    };
    ($router:path, $provider:expr $(,)?) => {
        #[cfg(all(target_arch = "wasm32", target_os = "wasi"))]
        const _: () = {
            use $crate::omnia_wasi_otel;

            struct MessagingGuest;

            $crate::omnia_wasi_messaging::export!(
                MessagingGuest with_types_in $crate::omnia_wasi_messaging
            );

            impl $crate::omnia_wasi_messaging::incoming_handler::Guest for MessagingGuest {
                #[omnia_wasi_otel::instrument(name = "messaging_guest_handle")]
                async fn handle(
                    message: $crate::omnia_wasi_messaging::types::Message,
                ) -> ::core::result::Result<(), $crate::omnia_wasi_messaging::types::Error>
                {
                    let router = $router($provider).await;
                    $crate::api::messaging::handle(&router, message).await
                }
            }
        };
    };
}

/// Bind an async router fn as the guest's `wasi:messaging` export.
///
/// Awaits the router fn on each delivery with [`WasiProvider`](crate::WasiProvider),
/// or with the provider expression passed as a second argument, and dispatches
/// the delivery through the resulting [`Router`]. Expands to nothing on native
/// targets, so the same crate's tests build the router natively.
///
/// ```rust,ignore
/// use omnia_sdk::api::messaging;
///
/// messaging::export!(messaging_router);
/// ```
#[doc(inline)]
pub use crate::__export_messaging as export;

/// An owned inbound delivery independent of a messaging binding.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Delivery {
    /// The exact topic supplied by the host.
    pub topic: Option<String>,
    /// The opaque message payload.
    pub payload: Vec<u8>,
    /// The optional payload media type.
    pub content_type: Option<String>,
    /// Transport metadata in delivery order.
    pub metadata: Vec<(String, String)>,
}

/// A delivery failure projected onto the current WIT error result.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DeliveryError {
    /// The delivery had no topic.
    MissingTopic,
    /// No route was registered for the exact topic.
    UnhandledTopic(String),
    /// Decoding or the handler rejected the delivery.
    Rejected(String),
}

impl fmt::Display for DeliveryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingTopic => f.write_str("message is missing topic"),
            Self::UnhandledTopic(topic) => write!(f, "unhandled topic: {topic}"),
            Self::Rejected(error) => f.write_str(error),
        }
    }
}

impl Error for DeliveryError {}

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
        Box::pin(async move {
            let input = (self.decode)(delivery)
                .map_err(|error| DeliveryError::Rejected(error.to_string()))?;
            let metadata = Metadata::from_lookup(|name| {
                delivery
                    .metadata
                    .iter()
                    .find(|(key, _)| key.eq_ignore_ascii_case(name))
                    .map(|(_, value)| value.clone())
            });
            client
                .call(self.handler.clone(), input, &metadata)
                .await
                .map(|_| ())
                .map_err(|error| DeliveryError::Rejected(error.to_string()))
        })
    }
}

/// Exact-topic routes awaiting the client that dispatches them.
///
/// Two routes for one topic are refused when they meet, whether through
/// [`route`](Self::route), [`merge`](Self::merge), `Routes::from([..])`, or
/// [`map`](Self::map).
#[must_use = "routes dispatch nothing until closed into a `Router`"]
pub struct Routes<P: Send + Sync + 'static> {
    routes: BTreeMap<String, Arc<dyn ErasedRoute<P>>>,
}

impl<P: Send + Sync + 'static> Default for Routes<P> {
    fn default() -> Self {
        Self::new()
    }
}

impl<P: Send + Sync + 'static> Routes<P> {
    /// Create an empty set of routes.
    pub fn new() -> Self {
        Self {
            routes: BTreeMap::new(),
        }
    }

    /// Register one handler for one exact topic, decoding the payload as JSON.
    ///
    /// # Panics
    ///
    /// Panics when the topic is empty or already registered.
    pub fn route<F, I>(self, topic: impl Into<String>, handler: F) -> Self
    where
        F: Handler<P, I>,
        I: DeserializeOwned + Send + 'static,
    {
        self.route_with(topic, handler, |delivery: &Delivery| {
            serde_json::from_slice(&delivery.payload)
                .map_err(|error| DecodeError::new(format!("malformed JSON payload: {error}")))
        })
    }

    /// Register one handler for one exact topic with a decoder over the whole delivery.
    ///
    /// # Panics
    ///
    /// Panics when the topic is empty or already registered.
    pub fn route_with<F, I, D>(self, topic: impl Into<String>, handler: F, decode: D) -> Self
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

    /// Take every route of `other`.
    ///
    /// # Panics
    ///
    /// Panics when a topic of `other` is already registered.
    pub fn merge(self, other: Self) -> Self {
        other.routes.into_iter().fold(self, |routes, (topic, route)| routes.insert(topic, route))
    }

    /// Re-key every route through `map`, so routes registered under a bare
    /// name are mounted under the deployment's.
    ///
    /// # Panics
    ///
    /// Panics when `map` yields an empty topic or one topic for two routes.
    pub fn map(self, mut map: impl FnMut(&str) -> String) -> Self {
        self.routes
            .into_iter()
            .fold(Self::new(), |routes, (topic, route)| routes.insert(map(&topic), route))
    }

    /// Close the routes over `client` as the [`Router`] that dispatches them.
    pub const fn with_state(self, client: Client<P>) -> Router<P> {
        Router { client, routes: self }
    }

    fn insert(mut self, topic: String, route: Arc<dyn ErasedRoute<P>>) -> Self {
        assert!(!topic.is_empty(), "messaging topic cannot be empty");
        assert!(!self.routes.contains_key(&topic), "duplicate messaging topic `{topic}`");
        self.routes.insert(topic, route);
        self
    }
}

impl<P: Send + Sync + 'static, const N: usize> From<[Self; N]> for Routes<P> {
    fn from(parts: [Self; N]) -> Self {
        parts.into_iter().reduce(Self::merge).unwrap_or_default()
    }
}

/// An exact-topic messaging router.
#[must_use = "a router dispatches nothing until it handles a delivery"]
pub struct Router<P: Send + Sync + 'static> {
    client: Client<P>,
    routes: Routes<P>,
}

impl<P: Send + Sync + 'static> Router<P> {
    /// Create an empty router backed by a client.
    pub fn new(client: Client<P>) -> Self {
        Self {
            client,
            routes: Routes::new(),
        }
    }

    /// Register one handler for one exact topic, decoding the payload as JSON.
    ///
    /// # Panics
    ///
    /// Panics when the topic is empty or already registered.
    pub fn route<F, I>(mut self, topic: impl Into<String>, handler: F) -> Self
    where
        F: Handler<P, I>,
        I: DeserializeOwned + Send + 'static,
    {
        self.routes = self.routes.route(topic, handler);
        self
    }

    /// Register one handler for one exact topic with a decoder over the whole delivery.
    ///
    /// # Panics
    ///
    /// Panics when the topic is empty or already registered.
    pub fn route_with<F, I, D>(mut self, topic: impl Into<String>, handler: F, decode: D) -> Self
    where
        F: Handler<P, I>,
        I: Send + 'static,
        D: Fn(&Delivery) -> Result<I, DecodeError> + Send + Sync + 'static,
    {
        self.routes = self.routes.route_with(topic, handler, decode);
        self
    }

    /// Take every route of `routes`.
    ///
    /// # Panics
    ///
    /// Panics when a topic of `routes` is already registered.
    pub fn merge(mut self, routes: Routes<P>) -> Self {
        self.routes = self.routes.merge(routes);
        self
    }

    /// The exact topics registered on this router, in order.
    pub fn topics(&self) -> impl Iterator<Item = &str> {
        self.routes.routes.keys().map(String::as_str)
    }

    /// Dispatch one delivery by exact topic.
    ///
    /// # Errors
    ///
    /// Returns missing-topic, unhandled-topic, decoding, or handler failures.
    pub async fn handle(&self, delivery: Delivery) -> Result<(), DeliveryError> {
        let topic = delivery.topic.as_deref().ok_or(DeliveryError::MissingTopic)?;
        let route = self
            .routes
            .routes
            .get(topic)
            .ok_or_else(|| DeliveryError::UnhandledTopic(topic.to_owned()))?;
        route.dispatch(&delivery, &self.client).await
    }
}

/// Adapt a WIT message to an owned delivery and dispatch it.
///
/// The current WIT contract carries only `result<_, error>`: success
/// acknowledges the delivery, while every router failure is returned as
/// `error.other` for the host to interpret.
///
/// # Errors
///
/// Returns the WIT delivery failure.
#[cfg(all(target_arch = "wasm32", target_os = "wasi"))]
pub async fn handle<P: Send + Sync + 'static>(
    router: &Router<P>, message: omnia_wasi_messaging::types::Message,
) -> Result<(), omnia_wasi_messaging::types::Error> {
    let delivery = Delivery {
        topic: message.topic(),
        payload: message.data(),
        content_type: message.content_type(),
        metadata: message.metadata().unwrap_or_default(),
    };
    router
        .handle(delivery)
        .await
        .map_err(|error| omnia_wasi_messaging::types::Error::Other(error.to_string()))
}
