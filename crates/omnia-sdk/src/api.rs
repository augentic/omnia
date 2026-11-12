//! Transport-neutral handler invocation and transport adapters.
//!
//! Application logic is a [`Handler`]: any `async fn(Context<P>, I)`
//! returning a `Result`. A module registers its HTTP paths, messaging
//! topics, and subcommands on a [`Handlers`] of its own and the guest merges
//! them (`Handlers::from([..])` or [`Handlers::merge`]), closing the whole
//! with [`Handlers::with_state`] into a [`Router`] that serves HTTP
//! requests, dispatches messaging deliveries, and runs argv. The [`Client`]
//! the router closes over owns the provider, and the router's dispatch
//! builds each handler's [`Context`] over it. [`Context::new`] builds one
//! directly to unit-test a handler. Each transport's `export!` macro binds
//! the router as the guest's WASI export.
//!
//! ```rust,ignore
//! use std::convert::Infallible;
//!
//! use omnia_sdk::api::{Client, Context, Handlers, Router};
//! use serde::Deserialize;
//!
//! struct Provider;
//!
//! #[derive(Deserialize)]
//! struct Greet {
//!     name: String,
//! }
//!
//! async fn greet(context: Context<Provider>, input: Greet) -> Result<String, Infallible> {
//!     Ok(format!("hello, {} ({:?})", input.name, context.metadata.request_id))
//! }
//!
//! fn router(provider: Provider) -> Router<Provider> {
//!     Handlers::new().topic("greet", greet).with_state(Client::new(provider))
//! }
//! ```

use std::error::Error;
use std::fmt;
use std::future::Future;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

pub mod command;
#[cfg(feature = "http")]
pub mod http;
pub mod messaging;

// What `Handlers` and `Router` hold for HTTP when the transport is not built.
#[cfg(not(feature = "http"))]
mod http {
    use std::fmt;
    use std::marker::PhantomData;

    use super::Client;

    pub(super) struct Routes<P>(PhantomData<fn() -> P>);

    impl<P> fmt::Debug for Routes<P> {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str("Routes")
        }
    }

    impl<P> Routes<P> {
        pub(super) const fn new() -> Self {
            Self(PhantomData)
        }

        pub(super) const fn merge(self, _other: Self) -> Self {
            self
        }

        #[expect(clippy::unused_self, reason = "the signature `axum::Router::with_state` has")]
        pub(super) fn with_state(self, _client: Client<P>) -> Served {
            Served
        }
    }

    impl<P> Clone for Routes<P> {
        fn clone(&self) -> Self {
            Self(PhantomData)
        }
    }

    #[derive(Clone, Debug)]
    pub(super) struct Served;
}

use self::command::{Commands, Program};
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

/// Transport-neutral invocation metadata.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
#[non_exhaustive]
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
    ///
    /// The Omnia host sets it on every HTTP request and messaging delivery, to
    /// the instant its guest timeout expires. A command runs unbounded and
    /// carries none.
    pub deadline: Option<SystemTime>,
}

impl Metadata {
    /// Build metadata from a transport's named-value lookup.
    ///
    /// Names are the transport-neutral `request-id` / `correlation-id` /
    /// `causation-id` / `tenant` / `deadline`. A missing request id is minted
    /// so every invocation is observable; the correlation id falls back to the
    /// request id. The `deadline` is decimal milliseconds since the Unix epoch,
    /// and a value that does not parse reads as none.
    ///
    /// # Panics
    ///
    /// Panics if the platform's random source fails while minting a request id.
    pub fn from_lookup(lookup: impl Fn(&str) -> Option<String>) -> Self {
        let request_id = lookup("request-id").unwrap_or_else(mint_request_id);
        Self {
            correlation_id: Some(lookup("correlation-id").unwrap_or_else(|| request_id.clone())),
            request_id: Some(request_id),
            causation_id: lookup("causation-id"),
            tenant: lookup("tenant"),
            deadline: lookup("deadline").as_deref().and_then(parse_deadline),
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

// A fresh request id: 32 lowercase hex chars from 128 bits of the platform's
// CSPRNG, which is `wasi:random` in a guest and the OS on native.
fn mint_request_id() -> String {
    let mut bytes = [0_u8; 16];
    getrandom::fill(&mut bytes).expect("the platform's random source is available");
    format!("{:032x}", u128::from_be_bytes(bytes))
}

fn parse_deadline(millis: &str) -> Option<SystemTime> {
    SystemTime::UNIX_EPOCH.checked_add(Duration::from_millis(millis.parse().ok()?))
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

// one accessor per capability, so a handler bounded on two traits that both
// define a method (`Config::get`, `StateStore::get`) names the one it means
// without UFCS
macro_rules! accessors {
    ($($(#[$meta:meta])* $name:ident: $trait:ident),* $(,)?) => {
        $(
            $(#[$meta])*
            impl<P: crate::$trait> Context<P> {
                #[doc = concat!(
                    "Borrow the provider as a [`", stringify!($trait), "`](crate::",
                    stringify!($trait), ")."
                )]
                #[must_use]
                pub fn $name(&self) -> &impl crate::$trait {
                    &*self.provider
                }
            }
        )*
    };
}

accessors! {
    blobs: BlobStore,
    config: Config,
    docs: DocumentDb,
    http: HttpClient,
    identity: Identity,
    model: Model,
    plugins: Plugins,
    publisher: Publisher,
    secrets: SecretStore,
    sql: SqlDb,
    state: StateStore,
    vcs: Vcs,
    websockets: WebSockets,
}

/// An application handler: any `async fn(Context<P>, I) -> Result<O, E>`.
///
/// The context comes first, as it does for an HTTP handler that takes
/// [`Context`] as its leading extractor. Every such fn (and every `Clone`
/// closure of that shape) is a `Handler` through the blanket impl; implement
/// the trait by hand only for a non-fn type.
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
        self, context: Context<P>, input: I,
    ) -> impl Future<Output = Result<Self::Output, Self::Error>> + Send;
}

// `I` is a trait parameter rather than an associated type: an associated
// `Input` would be unconstrained in this blanket impl (E0207).
impl<F, Fut, P, I, O, E> Handler<P, I> for F
where
    F: FnOnce(Context<P>, I) -> Fut + Clone + Send + Sync + 'static,
    Fut: Future<Output = Result<O, E>> + Send,
    O: Send,
    E: Error + Send + Sync + 'static,
{
    type Error = E;
    type Output = O;

    fn call(
        self, context: Context<P>, input: I,
    ) -> impl Future<Output = Result<Self::Output, Self::Error>> + Send {
        self(context, input)
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

    // Opens no `handler` span; the transport's dispatch does.
    pub(crate) async fn call<F, I>(
        &self, handler: F, input: I, metadata: &Metadata,
    ) -> Result<F::Output, F::Error>
    where
        F: Handler<P, I>,
    {
        let context = Context {
            provider: Arc::clone(&self.provider),
            metadata: metadata.clone(),
        };
        handler.call(context, input).await
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

/// HTTP routes, messaging topics, and subcommands over a provider-owning
/// [`Client`], awaiting their state.
///
/// A module registers its paths, topics, and subcommands on one of these and
/// the guest merges them (`Handlers::from([..])` or [`merge`](Self::merge)),
/// closing the whole with [`with_state`](Self::with_state) into a [`Router`].
/// Two handlers for one path and method, one topic, or one subcommand are
/// refused when they meet.
#[must_use = "handlers serve nothing until closed into a `Router` with `with_state`"]
pub struct Handlers<P: Send + Sync + 'static> {
    http: http::Routes<P>,
    topics: Topics<P>,
    commands: Commands<P>,
}

impl<P: Send + Sync + 'static> Handlers<P> {
    /// Create an empty set of handlers.
    pub fn new() -> Self {
        Self {
            http: http::Routes::new(),
            topics: Topics::new(),
            commands: Commands::new(),
        }
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

    /// Take every route, topic, and subcommand of `other`.
    ///
    /// # Panics
    ///
    /// Panics when a path of `other` overlaps an existing route for the same
    /// method, or a topic or subcommand of `other` is already registered.
    pub fn merge(self, other: Self) -> Self {
        Self {
            http: self.http.merge(other.http),
            topics: self.topics.merge(other.topics),
            commands: self.commands.merge(other.commands),
        }
    }

    /// Close the handlers over `client` as the [`Router`] that serves HTTP
    /// requests, dispatches messaging deliveries, and runs argv.
    pub fn with_state(self, client: Client<P>) -> Router<P> {
        Router {
            http: self.http.with_state(client.clone()),
            client,
            topics: self.topics,
            program: Program::new(self.commands),
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
            http: self.http.clone(),
            topics: self.topics.clone(),
            commands: self.commands.clone(),
        }
    }
}

impl<P: Send + Sync + 'static> fmt::Debug for Handlers<P> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Handlers")
            .field("http", &self.http)
            .field("topics", &self.topics)
            .field("commands", &self.commands)
            .finish()
    }
}

/// A finished router backed by a [`Client`], ready to serve HTTP requests,
/// dispatch messaging deliveries, and run argv.
///
/// [`Handlers::with_state`] is the one way to build it: every route, topic,
/// and subcommand is registered on a [`Handlers`] first. With the `command`
/// feature the router is also the program clap shows around its subcommands:
/// [`name`](Self::name), [`version`](Self::version), [`about`](Self::about),
/// and [`env_prefix`](Self::env_prefix) set what `--help`, `--version`, and
/// [`Metadata::from_env`] read, and [`hints`](Self::hints) the remedy every
/// failure may carry.
#[must_use = "a router serves nothing until it handles a request or delivery or runs argv"]
pub struct Router<P: Send + Sync + 'static = ()> {
    client: Client<P>,
    http: http::Served,
    topics: Topics<P>,
    program: Program<P>,
}

impl<P: Send + Sync + 'static> Router<P> {
    /// The exact topics registered on this router, in order.
    pub fn topics(&self) -> impl Iterator<Item = &str> {
        self.topics.topics()
    }

    /// Dispatch one delivery by exact topic.
    ///
    /// A broker serving the router acts on a failure by its variant:
    ///
    /// - It drops an [`Undecodable`](DeliveryError::Undecodable) delivery,
    ///   which no redelivery decodes.
    /// - It redelivers a [`Failed`](DeliveryError::Failed) one of kind
    ///   [`Unavailable`](crate::capability::ErrorKind::Unavailable), which may
    ///   succeed later.
    ///
    /// # Errors
    ///
    /// Returns [`DeliveryError::MissingTopic`] or
    /// [`DeliveryError::UnhandledTopic`] when no route takes the delivery,
    /// [`DeliveryError::Undecodable`] when its payload does not decode, and
    /// [`DeliveryError::Failed`] when the handler fails.
    pub async fn handle(&self, delivery: Delivery) -> Result<(), DeliveryError> {
        self.topics.handle(delivery, &self.client).await
    }
}

impl<P: Send + Sync + 'static> Clone for Router<P> {
    fn clone(&self) -> Self {
        Self {
            client: self.client.clone(),
            http: self.http.clone(),
            topics: self.topics.clone(),
            program: self.program.clone(),
        }
    }
}

impl<P: Send + Sync + 'static> fmt::Debug for Router<P> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Router")
            .field("http", &self.http)
            .field("topics", &self.topics)
            .field("program", &self.program)
            .finish_non_exhaustive()
    }
}
