//! Transport-neutral handler invocation and transport adapters.
//!
//! Application logic is a [`Handler`]: any `async fn(I, Context<P>)`
//! returning a `Result`. [`Client`] owns the provider and builds the
//! [`Context`] each call receives. A module registers its HTTP paths and
//! messaging topics on a [`Handlers`] of its own and the guest merges them
//! (`Handlers::from([..])` or [`Handlers::merge`]), closing the whole with
//! [`Handlers::with_state`] into a [`Router`] that serves HTTP requests and
//! dispatches messaging deliveries; [`command::Commands`] does the same for
//! subcommands. Each transport's `export!` macro binds its router or entry as
//! the guest's WASI export.
//!
//! ```rust,ignore
//! use std::convert::Infallible;
//!
//! use omnia_sdk::api::{Client, Context, Metadata};
//!
//! struct Provider;
//!
//! struct Greet {
//!     name: String,
//! }
//!
//! async fn greet(input: Greet, context: Context<Provider>) -> Result<String, Infallible> {
//!     Ok(format!("hello, {} ({:?})", input.name, context.metadata.request_id))
//! }
//!
//! async fn run() -> String {
//!     let client = Client::new(Provider);
//!     client.call(greet, Greet { name: "omnia".into() }, &Metadata::default()).await.unwrap()
//! }
//! ```

use std::error::Error;
use std::fmt;
use std::future::Future;
use std::sync::Arc;
use std::time::SystemTime;

#[cfg(feature = "http")]
use axum::routing::MethodRouter;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use tracing::Instrument as _;

pub mod command;
#[cfg(feature = "http")]
pub mod http;
pub mod messaging;

use self::messaging::{Delivery, DeliveryError, Topics};

/// An input that could not be decoded into handler input.
#[derive(Debug)]
pub struct DecodeError {
    description: String,
}

impl DecodeError {
    /// Create a decode error from its description.
    pub fn new(description: impl Into<String>) -> Self {
        Self {
            description: description.into(),
        }
    }

    /// Describe why the input could not be decoded.
    #[must_use]
    pub fn description(&self) -> &str {
        &self.description
    }
}

impl fmt::Display for DecodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.description)
    }
}

impl Error for DecodeError {}

/// The transport-neutral wire body of a failed invocation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ErrorBody {
    /// The error code discriminant.
    pub error: String,

    /// The human-readable error description.
    pub message: String,
}

impl From<&crate::Error> for ErrorBody {
    fn from(error: &crate::Error) -> Self {
        Self {
            error: error.code(),
            message: error.description(),
        }
    }
}

/// The wire format a handler output is encoded into.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[cfg_attr(feature = "command", derive(clap::ValueEnum))]
pub enum Format {
    /// Human-readable text produced by a render fn.
    Text,

    /// Pretty-printed JSON with a trailing newline.
    Json,
}

impl Format {
    /// Encode a body, rendering it through `render` for [`Format::Text`].
    ///
    /// # Panics
    ///
    /// Panics if `render` reports an error (a `String` sink never does) or
    /// `body` fails to serialize; both are invariant violations for a
    /// derived DTO encoded into memory, not runtime conditions.
    pub fn encode<T: Serialize>(
        self, body: &T, render: impl Fn(&T, &mut dyn fmt::Write) -> fmt::Result,
    ) -> Encoded {
        match self {
            Self::Text => {
                let mut out = String::new();
                render(body, &mut out).expect("a String sink never fails");
                Encoded {
                    bytes: out.into_bytes(),
                    media_type: "text/plain; charset=utf-8",
                }
            }
            Self::Json => {
                let mut bytes = serde_json::to_vec_pretty(body).expect("a derived DTO serializes");
                bytes.push(b'\n');
                Encoded {
                    bytes,
                    media_type: "application/json",
                }
            }
        }
    }
}

/// An encoded body with its media type.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Encoded {
    /// The encoded body bytes.
    pub bytes: Vec<u8>,

    /// The media type describing `bytes`.
    pub media_type: &'static str,
}

/// Transport-neutral invocation metadata.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Metadata {
    /// Identifies this invocation at its transport boundary.
    pub request_id: Option<String>,

    /// Correlates work across transport and capability boundaries.
    pub correlation_id: Option<String>,

    /// Identifies the invocation that directly caused this work.
    pub causation_id: Option<String>,

    /// The tenant the work is done for, when the transport names one.
    pub tenant: Option<String>,

    /// The latest instant at which the caller considers the work useful.
    pub deadline: Option<SystemTime>,
}

impl Metadata {
    /// Build metadata from a transport's named-value lookup.
    ///
    /// Names are the transport-neutral `request-id` / `correlation-id` /
    /// `causation-id` / `tenant`. A missing request id is minted so every
    /// invocation is observable; the correlation id falls back to the
    /// request id.
    pub fn from_lookup(lookup: impl Fn(&str) -> Option<String>) -> Self {
        let request_id = lookup("request-id").unwrap_or_else(mint_request_id);
        Self {
            correlation_id: Some(lookup("correlation-id").unwrap_or_else(|| request_id.clone())),
            request_id: Some(request_id),
            causation_id: lookup("causation-id"),
            tenant: lookup("tenant"),
            deadline: None,
        }
    }

    /// Mint metadata for a transport-initiated invocation.
    ///
    /// The freshly minted request id doubles as the correlation id.
    #[must_use]
    pub fn minted(request_id: String) -> Self {
        Self {
            correlation_id: Some(request_id.clone()),
            request_id: Some(request_id),
            causation_id: None,
            tenant: None,
            deadline: None,
        }
    }
}

// A fresh request id: 32 lowercase hex chars from 128 random bits.
fn mint_request_id() -> String {
    let (high, low) = random_u64_pair();
    format!("{high:016x}{low:016x}")
}

// The native arm exists so the transports are testable off-target; it
// is unique per call but not a CSPRNG, and no native transport mints ids.
fn random_u64_pair() -> (u64, u64) {
    cfg_select! {
        all(target_arch = "wasm32", target_os = "wasi") => {
            use wasip3::random::random::get_random_u64;

            (get_random_u64(), get_random_u64())
        }
        _ => {
            use std::hash::{BuildHasher, RandomState};

            (RandomState::new().hash_one(0_u8), RandomState::new().hash_one(1_u8))
        }
    }
}

/// Context owned by one handler call.
///
/// With the `http` feature it is also an axum extractor over the [`Client`]
/// in the router state, so an HTTP handler is `async fn(Context<P>, ..)`.
#[derive(Clone, Debug)]
pub struct Context<P> {
    pub(crate) provider: Arc<P>,

    /// Transport-neutral invocation metadata.
    pub metadata: Metadata,
}

impl<P> Context<P> {
    /// Create a context outside a [`Client`], typically to unit-test a handler.
    pub fn new(provider: P, metadata: Metadata) -> Self {
        Self {
            provider: Arc::new(provider),
            metadata,
        }
    }

    /// Return the provider used to fulfil the call.
    #[must_use]
    pub fn provider(&self) -> &P {
        &self.provider
    }
}

/// An application handler: any `async fn(I, Context<P>) -> Result<O, E>`.
///
/// Every such fn (and every `Clone` closure of that shape) is a `Handler`
/// through the blanket impl; implement the trait by hand only for a
/// non-fn type.
pub trait Handler<P, I>: Clone + Send + Sync + 'static {
    /// The typed handler output.
    type Output: Send;

    /// The handler failure.
    type Error: Error + Send + Sync + 'static;

    /// Execute the handler.
    ///
    /// # Errors
    ///
    /// Returns the handler's error.
    fn call(
        self, input: I, context: Context<P>,
    ) -> impl Future<Output = Result<Self::Output, Self::Error>> + Send;
}

// `I` is a trait parameter rather than an associated type: an associated
// `Input` would be unconstrained in this blanket impl (E0207).
impl<F, Fut, P, I, O, E> Handler<P, I> for F
where
    F: FnOnce(I, Context<P>) -> Fut + Clone + Send + Sync + 'static,
    Fut: Future<Output = Result<O, E>> + Send,
    O: Send,
    E: Error + Send + Sync + 'static,
{
    type Error = E;
    type Output = O;

    fn call(
        self, input: I, context: Context<P>,
    ) -> impl Future<Output = Result<Self::Output, Self::Error>> + Send {
        self(input, context)
    }
}

/// Provider-owning handler client.
///
/// Clones share one provider allocation. Transports define its lifetime; HTTP
/// constructs one client per WASI request and keeps durable state host-side.
pub struct Client<P> {
    pub(crate) provider: Arc<P>,
}

impl<P: Send + Sync + 'static> Client<P> {
    /// Create a client with one clone-shared provider allocation.
    pub fn new(provider: P) -> Self {
        Self {
            provider: Arc::new(provider),
        }
    }

    /// Return the shared provider.
    #[must_use]
    pub fn provider(&self) -> &P {
        &self.provider
    }

    /// Invoke a handler with the given input and metadata inside a `handler`
    /// tracing span carrying the request and correlation ids.
    ///
    /// # Errors
    ///
    /// Returns the handler's error.
    pub async fn call<F, I>(
        &self, handler: F, input: I, metadata: &Metadata,
    ) -> Result<F::Output, F::Error>
    where
        F: Handler<P, I>,
    {
        let span = tracing::info_span!(
            "handler",
            request_id = metadata.request_id.as_deref(),
            correlation_id = metadata.correlation_id.as_deref(),
        );
        let context = Context {
            provider: Arc::clone(&self.provider),
            metadata: metadata.clone(),
        };
        handler.call(input, context).instrument(span).await
    }
}

impl<P: Send + Sync + 'static> Clone for Client<P> {
    fn clone(&self) -> Self {
        Self {
            provider: Arc::clone(&self.provider),
        }
    }
}

impl<P: Send + Sync + 'static> fmt::Debug for Client<P> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Client").finish_non_exhaustive()
    }
}

/// HTTP routes and messaging topics over a provider-owning [`Client`], awaiting their state.
///
/// A module registers its paths and topics on one of these and the guest
/// merges them (`Handlers::from([..])` or [`merge`](Self::merge)), closing the
/// whole with [`with_state`](Self::with_state) into a [`Router`]. Two handlers
/// for one path and method, or for one topic, are refused when they meet.
#[must_use = "handlers serve nothing until closed into a `Router` with `with_state`"]
pub struct Handlers<P: Send + Sync + 'static> {
    #[cfg(feature = "http")]
    http: axum::Router<Client<P>>,
    topics: Topics<P>,
}

impl<P: Send + Sync + 'static> Handlers<P> {
    /// Create an empty set of handlers.
    pub fn new() -> Self {
        Self {
            #[cfg(feature = "http")]
            http: axum::Router::new(),
            topics: Topics::new(),
        }
    }

    /// Register a [`MethodRouter`] at `path`.
    ///
    /// # Panics
    ///
    /// Panics when `path` overlaps an existing route for the same method.
    #[cfg(feature = "http")]
    pub fn route(mut self, path: &str, method_router: MethodRouter<Client<P>>) -> Self {
        self.http = self.http.route(path, method_router);
        self
    }

    /// Register one handler for one exact topic, decoding the payload as JSON.
    ///
    /// # Panics
    ///
    /// Panics when the topic is empty or already registered.
    pub fn topic<F, I>(mut self, topic: impl Into<String>, handler: F) -> Self
    where
        F: Handler<P, I>,
        I: DeserializeOwned + Send + 'static,
    {
        self.topics = self.topics.route(topic, handler);
        self
    }

    /// Register one handler for one exact topic with a decoder over the whole delivery.
    ///
    /// # Panics
    ///
    /// Panics when the topic is empty or already registered.
    pub fn topic_with<F, I, D>(mut self, topic: impl Into<String>, handler: F, decode: D) -> Self
    where
        F: Handler<P, I>,
        I: Send + 'static,
        D: Fn(&Delivery) -> Result<I, DecodeError> + Send + Sync + 'static,
    {
        self.topics = self.topics.route_with(topic, handler, decode);
        self
    }

    /// Take every route and topic of `other`.
    ///
    /// # Panics
    ///
    /// Panics when a path of `other` overlaps an existing route for the same
    /// method or a topic of `other` is already registered.
    pub fn merge(self, other: Self) -> Self {
        Self {
            #[cfg(feature = "http")]
            http: self.http.merge(other.http),
            topics: self.topics.merge(other.topics),
        }
    }

    /// Re-key every topic through `map`, so topics registered under a bare
    /// name are mounted under the deployment's.
    ///
    /// # Panics
    ///
    /// Panics when `map` yields an empty topic or one topic for two routes.
    pub fn map_topics(mut self, map: impl FnMut(&str) -> String) -> Self {
        self.topics = self.topics.map(map);
        self
    }

    /// Close the handlers over `client` as the [`Router`] that serves HTTP
    /// requests and dispatches messaging deliveries.
    pub fn with_state(self, client: Client<P>) -> Router<P> {
        Router {
            #[cfg(feature = "http")]
            http: self.http.with_state(client.clone()),
            client,
            topics: self.topics,
        }
    }
}

impl<P: Send + Sync + 'static> Default for Handlers<P> {
    fn default() -> Self {
        Self::new()
    }
}

impl<P: Send + Sync + 'static, const N: usize> From<[Self; N]> for Handlers<P> {
    fn from(parts: [Self; N]) -> Self {
        parts.into_iter().reduce(Self::merge).unwrap_or_default()
    }
}

impl<P: Send + Sync + 'static> Clone for Handlers<P> {
    fn clone(&self) -> Self {
        Self {
            #[cfg(feature = "http")]
            http: self.http.clone(),
            topics: self.topics.clone(),
        }
    }
}

impl<P: Send + Sync + 'static> fmt::Debug for Handlers<P> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut debug = f.debug_struct("Handlers");
        #[cfg(feature = "http")]
        debug.field("http", &self.http);
        debug.field("topics", &self.topics).finish()
    }
}

#[cfg(feature = "http")]
impl<P: Send + Sync + 'static> From<Handlers<P>> for axum::Router<Client<P>> {
    fn from(handlers: Handlers<P>) -> Self {
        handlers.http
    }
}

/// A finished router backed by a [`Client`], ready to serve HTTP requests and
/// dispatch messaging deliveries.
#[must_use = "a router serves nothing until it handles a request or delivery"]
pub struct Router<P: Send + Sync + 'static = ()> {
    client: Client<P>,
    #[cfg(feature = "http")]
    http: axum::Router,
    topics: Topics<P>,
}

impl<P: Send + Sync + 'static> Router<P> {
    /// Create an empty router backed by a client.
    pub fn new(client: Client<P>) -> Self {
        Handlers::new().with_state(client)
    }

    /// Register a [`MethodRouter`] at `path`.
    ///
    /// # Panics
    ///
    /// Panics when `path` overlaps an existing route for the same method.
    #[cfg(feature = "http")]
    pub fn route(mut self, path: &str, method_router: MethodRouter<Client<P>>) -> Self {
        self.http = self.http.route(path, method_router.with_state(self.client.clone()));
        self
    }

    /// Apply a [`tower::Layer`] to every HTTP route on this router.
    #[cfg(feature = "http")]
    pub fn layer<L>(mut self, layer: L) -> Self
    where
        L: tower::Layer<axum::routing::Route> + Clone + Send + Sync + 'static,
        L::Service:
            tower::Service<::http::Request<axum::body::Body>> + Clone + Send + Sync + 'static,
        <L::Service as tower::Service<::http::Request<axum::body::Body>>>::Response:
            axum::response::IntoResponse + 'static,
        <L::Service as tower::Service<::http::Request<axum::body::Body>>>::Error:
            Into<std::convert::Infallible> + 'static,
        <L::Service as tower::Service<::http::Request<axum::body::Body>>>::Future: Send + 'static,
    {
        self.http = self.http.layer(layer);
        self
    }

    /// Register one handler for one exact topic, decoding the payload as JSON.
    ///
    /// # Panics
    ///
    /// Panics when the topic is empty or already registered.
    pub fn topic<F, I>(mut self, topic: impl Into<String>, handler: F) -> Self
    where
        F: Handler<P, I>,
        I: DeserializeOwned + Send + 'static,
    {
        self.topics = self.topics.route(topic, handler);
        self
    }

    /// Register one handler for one exact topic with a decoder over the whole delivery.
    ///
    /// # Panics
    ///
    /// Panics when the topic is empty or already registered.
    pub fn topic_with<F, I, D>(mut self, topic: impl Into<String>, handler: F, decode: D) -> Self
    where
        F: Handler<P, I>,
        I: Send + 'static,
        D: Fn(&Delivery) -> Result<I, DecodeError> + Send + Sync + 'static,
    {
        self.topics = self.topics.route_with(topic, handler, decode);
        self
    }

    /// Take every route and topic of `handlers`.
    ///
    /// # Panics
    ///
    /// Panics when a path of `handlers` overlaps an existing route for the same
    /// method or a topic of `handlers` is already registered.
    pub fn merge(mut self, handlers: Handlers<P>) -> Self {
        #[cfg(feature = "http")]
        {
            self.http = self.http.merge(handlers.http.with_state(self.client.clone()));
        }
        self.topics = self.topics.merge(handlers.topics);
        self
    }

    /// The exact topics registered on this router, in order.
    pub fn topics(&self) -> impl Iterator<Item = &str> {
        self.topics.topics()
    }

    /// Dispatch one delivery by exact topic.
    ///
    /// # Errors
    ///
    /// Returns missing-topic, unhandled-topic, decoding, or handler failures.
    pub async fn handle(&self, delivery: Delivery) -> Result<(), DeliveryError> {
        self.topics.handle(delivery, &self.client).await
    }
}

impl<P: Send + Sync + 'static> Clone for Router<P> {
    fn clone(&self) -> Self {
        Self {
            client: self.client.clone(),
            #[cfg(feature = "http")]
            http: self.http.clone(),
            topics: self.topics.clone(),
        }
    }
}

impl<P: Send + Sync + 'static> fmt::Debug for Router<P> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut debug = f.debug_struct("Router");
        #[cfg(feature = "http")]
        debug.field("http", &self.http);
        debug.field("topics", &self.topics).finish_non_exhaustive()
    }
}

#[cfg(feature = "http")]
impl<P: Send + Sync + 'static> From<Router<P>> for axum::Router {
    fn from(router: Router<P>) -> Self {
        router.http
    }
}

#[cfg(feature = "http")]
impl<P, B> tower::Service<::http::Request<B>> for Router<P>
where
    P: Send + Sync + 'static,
    B: http_body::Body<Data = bytes::Bytes> + Send + 'static,
    B::Error: Into<axum::BoxError>,
{
    type Error = std::convert::Infallible;
    type Future = axum::routing::future::RouteFuture<std::convert::Infallible>;
    type Response = axum::response::Response;

    fn poll_ready(
        &mut self, cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Result<(), Self::Error>> {
        <axum::Router as tower::Service<::http::Request<B>>>::poll_ready(&mut self.http, cx)
    }

    fn call(&mut self, req: ::http::Request<B>) -> Self::Future {
        self.http.call(req)
    }
}
