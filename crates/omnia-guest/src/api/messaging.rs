//! Typed messaging routing over application operations.

use std::any::TypeId;
use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;
use std::future::Future;
use std::marker::PhantomData;
use std::pin::Pin;
use std::sync::Arc;

use serde::de::DeserializeOwned;

use crate::api::Provider;
use crate::api::invocation::{Invocation, Metadata};
use crate::api::invoke::Invoker;
use crate::api::operation::Operation;

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

/// Converts an inbound delivery into an operation input.
pub trait Decoder<I>: Clone + Send + Sync + 'static {
    /// The decoding failure.
    type Error: Error + Send + Sync + 'static;

    /// Decode one delivery.
    ///
    /// # Errors
    ///
    /// Returns a typed payload decoding failure.
    fn decode(&self, delivery: &Delivery) -> Result<I, Self::Error>;
}

impl<I, E, F> Decoder<I> for F
where
    E: Error + Send + Sync + 'static,
    F: Fn(&Delivery) -> Result<I, E> + Clone + Send + Sync + 'static,
{
    type Error = E;

    fn decode(&self, delivery: &Delivery) -> Result<I, Self::Error> {
        self(delivery)
    }
}

/// Decodes the delivery payload as JSON.
#[derive(Clone, Copy, Debug, Default)]
pub struct Json;

impl<I> Decoder<I> for Json
where
    I: DeserializeOwned,
{
    type Error = serde_json::Error;

    fn decode(&self, delivery: &Delivery) -> Result<I, Self::Error> {
        serde_json::from_slice(&delivery.payload)
    }
}

pub use crate::api::Outcome;

/// A delivery failure projected onto the current WIT error result.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DeliveryError {
    /// The delivery had no topic.
    MissingTopic,
    /// No route was registered for the exact topic.
    UnhandledTopic(String),
    /// Application-local projection rejected the delivery.
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

/// Maps one typed route outcome to acknowledgement or delivery failure.
pub trait Projector<T, O, D>: Clone + Send + Sync + 'static {
    /// Project the operation outcome.
    ///
    /// `Ok(())` acknowledges the current delivery. `Err` becomes the error
    /// arm of the current `wasi:messaging/incoming-handler.handle` result.
    ///
    /// # Errors
    ///
    /// Returns a delivery failure for host-defined retry or rejection policy.
    fn project(&self, outcome: Outcome<T, O, D>) -> Result<(), DeliveryError>;
}

/// Acknowledges outputs and rejects typed operation or decoding failures.
#[derive(Clone, Copy, Debug, Default)]
pub struct Acknowledge;

impl<T, O, D> Projector<T, O, D> for Acknowledge
where
    O: fmt::Display,
    D: fmt::Display,
{
    fn project(&self, outcome: Outcome<T, O, D>) -> Result<(), DeliveryError> {
        match outcome {
            Outcome::Output(_) => Ok(()),
            Outcome::Operation(error) => Err(DeliveryError::Rejected(error.to_string())),
            Outcome::Decode(error) => Err(DeliveryError::Rejected(error.to_string())),
        }
    }
}

/// Begin a JSON-decoded, acknowledgement-projected route.
#[must_use]
pub fn consume<O>() -> Consume<O, Json, Acknowledge> {
    Consume {
        decoder: Json,
        projector: Acknowledge,
        marker: PhantomData,
    }
}

/// A typed messaging route before registration.
pub struct Consume<O, D, Q> {
    decoder: D,
    projector: Q,
    marker: PhantomData<fn() -> O>,
}

impl<O, D, Q> Consume<O, D, Q> {
    /// Replace the payload decoder policy.
    #[must_use]
    pub fn decode_with<D2>(self, decoder: D2) -> Consume<O, D2, Q> {
        Consume {
            decoder,
            projector: self.projector,
            marker: PhantomData,
        }
    }

    /// Replace the output and error delivery policy.
    #[must_use]
    pub fn project_with<Q2>(self, projector: Q2) -> Consume<O, D, Q2> {
        Consume {
            decoder: self.decoder,
            projector,
            marker: PhantomData,
        }
    }
}

type DispatchFuture<'a> = Pin<Box<dyn Future<Output = Result<(), DeliveryError>> + Send + 'a>>;

trait ErasedRoute<P: Provider>: Send + Sync {
    fn operation(&self) -> TypeId;
    fn dispatch<'a>(
        &'a self, delivery: &'a Delivery, invoker: &'a Invoker<P>,
    ) -> DispatchFuture<'a>;
}

struct Route<P, O, D, Q> {
    decoder: D,
    projector: Q,
    marker: PhantomData<fn(P) -> O>,
}

impl<P, O, D, Q> ErasedRoute<P> for Route<P, O, D, Q>
where
    P: Provider,
    O: Operation<P>,
    D: Decoder<O::Input>,
    Q: Projector<O::Output, O::Error, D::Error>,
{
    fn operation(&self) -> TypeId {
        TypeId::of::<O>()
    }

    fn dispatch<'a>(
        &'a self, delivery: &'a Delivery, invoker: &'a Invoker<P>,
    ) -> DispatchFuture<'a> {
        Box::pin(async move {
            let input = match self.decoder.decode(delivery) {
                Ok(input) => input,
                Err(error) => return self.projector.project(Outcome::Decode(error)),
            };
            let metadata = Metadata::from_lookup(|name| {
                delivery
                    .metadata
                    .iter()
                    .find(|(key, _)| key.eq_ignore_ascii_case(name))
                    .map(|(_, value)| value.clone())
            });
            let outcome = match invoker.invoke::<O>(Invocation::new(input).metadata(metadata)).await
            {
                Ok(output) => Outcome::Output(output),
                Err(error) => Outcome::Operation(error),
            };
            self.projector.project(outcome)
        })
    }
}

/// Read-only metadata for one exact topic registration.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RouteInfo {
    topic: String,
    operation: TypeId,
}

impl RouteInfo {
    /// Return the exact registered topic.
    #[must_use]
    pub fn topic(&self) -> &str {
        &self.topic
    }

    /// Return the process-local operation type identity.
    #[must_use]
    pub const fn operation(&self) -> TypeId {
        self.operation
    }
}

/// An exact-topic messaging router.
pub struct Router<P: Provider> {
    invoker: Invoker<P>,
    routes: BTreeMap<String, Arc<dyn ErasedRoute<P>>>,
    inventory: Vec<RouteInfo>,
}

impl<P: Provider> Router<P> {
    /// Create an empty router backed by an invoker.
    #[must_use]
    pub fn new(invoker: Invoker<P>) -> Self {
        Self {
            invoker,
            routes: BTreeMap::new(),
            inventory: Vec::new(),
        }
    }

    /// Register one operation for one exact topic.
    ///
    /// # Panics
    ///
    /// Panics when the topic is empty or already registered.
    #[must_use]
    pub fn route<O, D, Q>(mut self, topic: impl Into<String>, binding: Consume<O, D, Q>) -> Self
    where
        O: Operation<P>,
        D: Decoder<O::Input>,
        Q: Projector<O::Output, O::Error, D::Error>,
    {
        let topic = topic.into();
        assert!(!topic.is_empty(), "messaging topic cannot be empty");
        assert!(!self.routes.contains_key(&topic), "duplicate messaging topic `{topic}`");
        let route: Arc<dyn ErasedRoute<P>> = Arc::new(Route::<P, O, D, Q> {
            decoder: binding.decoder,
            projector: binding.projector,
            marker: PhantomData,
        });
        self.inventory.push(RouteInfo {
            topic: topic.clone(),
            operation: route.operation(),
        });
        self.routes.insert(topic, route);
        self
    }

    /// Return routes in registration order.
    #[must_use]
    pub fn inventory(&self) -> &[RouteInfo] {
        &self.inventory
    }

    /// Dispatch one delivery by exact topic.
    ///
    /// # Errors
    ///
    /// Returns missing-topic, unhandled-topic, decoding, operation, or
    /// application-local projection failures.
    pub async fn handle(&self, delivery: Delivery) -> Result<(), DeliveryError> {
        let topic = delivery.topic.as_deref().ok_or(DeliveryError::MissingTopic)?;
        let route = self
            .routes
            .get(topic)
            .ok_or_else(|| DeliveryError::UnhandledTopic(topic.to_owned()))?;
        route.dispatch(&delivery, &self.invoker).await
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
/// Returns the projected WIT delivery failure.
#[cfg(target_arch = "wasm32")]
pub async fn handle<P: Provider>(
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
