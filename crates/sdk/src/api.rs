//! Transport-neutral handler invocation and transport adapters.
//!
//! Application logic is a [`Handler`]: any `async fn(Context, I)` returning
//! a `Result`. A module registers its HTTP paths, messaging topics, and
//! subcommands on a [`Handlers`] of its own and the guest merges them
//! (`Handlers::from([..])` or [`Handlers::merge`]), closing the whole with
//! [`Handlers::into_router`] into a [`Router`] that serves HTTP requests,
//! dispatches messaging deliveries, and runs argv. A handler reaches the host
//! through the crate's bindings, and its [`Context`] carries the call's
//! [`Metadata`]. Each transport's `export!` macro binds the router fn as the
//! guest's WASI export.
//!
//! ```rust
//! use std::convert::Infallible;
//!
//! use omnia_sdk::api::{Context, Handlers, Router};
//! use serde::Deserialize;
//!
//! #[derive(Deserialize)]
//! struct Greet {
//!     name: String,
//! }
//!
//! async fn greet(context: Context, input: Greet) -> Result<String, Infallible> {
//!     Ok(format!("hello, {} ({:?})", input.name, context.metadata.request_id))
//! }
//!
//! fn router() -> Router {
//!     Handlers::new().topic("greet", greet).into_router()
//! }
//! ```

use std::error::Error;
use std::future::Future;
use std::time::{Duration, SystemTime};

use serde::de::DeserializeOwned;

pub mod command;
#[cfg(feature = "http")]
pub mod http;
pub mod messaging;

#[cfg(feature = "command")]
use self::command::program::{Commands, Program};
use self::messaging::{Delivery, Topics};

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
        let request_id = lookup("request-id").unwrap_or_else(request_id);

        Self {
            correlation_id: Some(lookup("correlation-id").unwrap_or_else(|| request_id.clone())),
            request_id: Some(request_id),
            causation_id: lookup("causation-id"),
            tenant: lookup("tenant"),
            deadline: lookup("deadline").and_then(|millis| {
                SystemTime::UNIX_EPOCH.checked_add(Duration::from_millis(millis.parse().ok()?))
            }),
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
fn request_id() -> String {
    let mut bytes = [0_u8; 16];
    getrandom::fill(&mut bytes).expect("the platform's random source is available");
    format!("{:032x}", u128::from_be_bytes(bytes))
}

/// Context owned by one handler call.
///
/// With the `http` feature it is also an axum extractor, so an HTTP handler
/// is `async fn(Context, ..)`.
#[derive(Clone, Debug, Default)]
pub struct Context {
    /// Transport-neutral invocation metadata.
    pub metadata: Metadata,
}

impl Context {
    /// A context carrying `metadata`, typically to unit-test a handler.
    #[must_use]
    pub const fn new(metadata: Metadata) -> Self {
        Self { metadata }
    }
}

/// An application handler: any `async fn(Context, I) -> Result<O, E>`.
///
/// The context comes first, as it does for an HTTP handler that takes
/// [`Context`] as its leading extractor. Every such fn (and every `Clone`
/// closure of that shape) is a `Handler` through the blanket impl; implement
/// the trait by hand only for a non-fn type.
pub trait Handler<I>: Clone + Send + Sync + 'static {
    /// The typed handler output.
    type Output: Send;

    /// The handler failure.
    ///
    /// A topic or subcommand reports it as an [`Error`](crate::Error):
    ///
    /// - The first [`Error`](crate::Error), [`vcs::Error`](crate::vcs::Error),
    ///   [`plugins::Error`](crate::plugins::Error), or
    ///   [`model::Error`](crate::model::Error) in its source chain sets the
    ///   variant and code.
    /// - Any other error is a [`ServerError`](crate::Error::ServerError) with
    ///   the code `server_error`.
    /// - The description is the whole chain.
    type Error: Error + Send + Sync + 'static;

    /// Execute the handler.
    ///
    /// # Errors
    ///
    /// Returns the handler's error.
    fn call(
        self, context: Context, input: I,
    ) -> impl Future<Output = Result<Self::Output, Self::Error>> + Send;
}

// `I` is a trait parameter rather than an associated type: an associated
// `Input` would be unconstrained in this blanket impl (E0207).
impl<F, Fut, I, O, E> Handler<I> for F
where
    F: FnOnce(Context, I) -> Fut + Clone + Send + Sync + 'static,
    Fut: Future<Output = Result<O, E>> + Send,
    O: Send,
    E: Error + Send + Sync + 'static,
{
    type Error = E;
    type Output = O;

    fn call(
        self, context: Context, input: I,
    ) -> impl Future<Output = Result<Self::Output, Self::Error>> + Send {
        self(context, input)
    }
}

/// HTTP routes, messaging topics, and subcommands awaiting their [`Router`].
///
/// A module registers its paths, topics, and subcommands on one of these and
/// the guest merges them (`Handlers::from([..])` or [`merge`](Self::merge)),
/// closing the whole with [`into_router`](Self::into_router). Two handlers
/// for one path and method, one topic, or one subcommand are refused when
/// they meet.
#[derive(Clone, Debug, Default)]
#[must_use = "handlers serve nothing until closed with `into_router`"]
pub struct Handlers {
    #[cfg(feature = "http")]
    http: axum::Router,
    topics: Topics,
    #[cfg(feature = "command")]
    commands: Commands,
}

impl Handlers {
    /// Create an empty set of handlers.
    pub fn new() -> Self {
        Self::default()
    }

    /// Register one handler for one exact topic, decoding the payload as JSON.
    ///
    /// # Panics
    ///
    /// Panics when the topic is empty or already registered.
    pub fn topic<F, I>(mut self, topic: impl Into<String>, handler: F) -> Self
    where
        F: Handler<I>,
        I: DeserializeOwned + Send + 'static,
    {
        self.topics = self.topics.route(topic, handler);
        self
    }

    /// Register one handler for one exact topic with a decoder over the whole delivery.
    ///
    /// A decoder's refusal fails the delivery as a
    /// [`BadRequest`](crate::Error::BadRequest) with the code `undecodable`,
    /// whatever its own variant and code, and keeps its description.
    ///
    /// # Panics
    ///
    /// Panics when the topic is empty or already registered.
    pub fn topic_with<F, I, D>(mut self, topic: impl Into<String>, handler: F, decode: D) -> Self
    where
        F: Handler<I>,
        I: Send + 'static,
        D: Fn(&Delivery) -> crate::Result<I> + Send + Sync + 'static,
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
            #[cfg(feature = "http")]
            http: self.http.merge(other.http),
            topics: self.topics.merge(other.topics),
            #[cfg(feature = "command")]
            commands: self.commands.merge(other.commands),
        }
    }

    /// Close the handlers into the [`Router`] that serves HTTP requests,
    /// dispatches messaging deliveries, and runs argv.
    pub fn into_router(self) -> Router {
        Router {
            #[cfg(feature = "http")]
            http: self.http,
            topics: self.topics,
            #[cfg(feature = "command")]
            program: Program::new(self.commands),
        }
    }
}

impl<const N: usize> From<[Self; N]> for Handlers {
    fn from(parts: [Self; N]) -> Self {
        parts.into_iter().reduce(Self::merge).unwrap_or_default()
    }
}

// links to `command`-only methods take an anchor, which resolves without the feature
/// A finished router, ready to serve HTTP requests, dispatch messaging
/// deliveries, and run argv.
///
/// [`Handlers::into_router`] is the one way to build it: every route, topic,
/// and subcommand is registered on a [`Handlers`] first. With the `command`
/// feature the router is also the program clap shows around its subcommands:
/// [`name`](Self#method.name), [`version`](Self#method.version),
/// [`about`](Self#method.about), and [`env_prefix`](Self#method.env_prefix)
/// set what `--help`, `--version`, and [`Metadata::from_env`] read, and
/// [`hints`](Self#method.hints) the remedy every failure may carry.
#[derive(Clone, Debug)]
#[must_use = "a router serves nothing until it handles a request or delivery or runs argv"]
pub struct Router {
    #[cfg(feature = "http")]
    http: axum::Router,
    topics: Topics,
    #[cfg(feature = "command")]
    program: Program,
}

impl Router {
    /// The exact topics registered on this router, in order.
    pub fn topics(&self) -> impl Iterator<Item = &str> {
        self.topics.topics()
    }

    /// Dispatch one delivery by exact topic.
    ///
    /// A broker serving the router acts on a failure by its code and variant:
    ///
    /// - It drops one with the code `undecodable`, which no redelivery decodes.
    /// - It redelivers a [`BadGateway`](crate::Error::BadGateway), which may
    ///   succeed later.
    ///
    /// # Errors
    ///
    /// - A [`BadRequest`](crate::Error::BadRequest) with the code
    ///   `missing-topic` when the delivery has no topic.
    /// - A [`NotFound`](crate::Error::NotFound) with the code `unhandled-topic`
    ///   when no route takes the topic.
    /// - A [`BadRequest`](crate::Error::BadRequest) with the code
    ///   `undecodable` when the payload does not decode.
    /// - The handler's error when it fails, as [`Handler::Error`] describes.
    pub async fn handle(&self, delivery: Delivery) -> crate::Result<()> {
        self.topics.handle(delivery).await
    }
}
