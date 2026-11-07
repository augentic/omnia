//! Transport-neutral handler invocation and transport adapters.
//!
//! Application logic is a [`Handler`]: any `async fn(Context<P>, I)`
//! returning a `Result`. [`Client`] owns the provider and builds the
//! [`Context`] each call receives. A module registers its HTTP paths,
//! messaging topics, and subcommands on a [`Handlers`] of its own and the
//! guest merges them (`Handlers::from([..])` or [`Handlers::merge`]), closing
//! the whole with [`Handlers::with_state`] into a [`Router`] that serves HTTP
//! requests, dispatches messaging deliveries, and runs argv. Each transport's
//! `export!` macro binds the router as the guest's WASI export.
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
//! async fn greet(context: Context<Provider>, input: Greet) -> Result<String, Infallible> {
//!     Ok(format!("hello, {} ({:?})", input.name, context.metadata.request_id))
//! }
//!
//! async fn run() -> String {
//!     let client = Client::new(Provider);
//!     client.call(greet, Greet { name: "omnia".into() }, &Metadata::default()).await.unwrap()
//! }
//! ```

#[cfg(feature = "command")]
use std::borrow::Cow;
use std::error::Error;
#[cfg(feature = "command")]
use std::ffi::OsString;
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

#[cfg(feature = "command")]
use self::command::{Commands, Failure, Program, Response, Shell};
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

/// The wire format a handler output is encoded into: the body's [`Render`]
/// form as text, its [`Serialize`] form as JSON.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[cfg_attr(feature = "command", derive(clap::ValueEnum))]
pub enum Format {
    /// Human-readable text.
    Text,

    /// Pretty-printed JSON with a trailing newline.
    Json,
}

impl Format {
    /// Encode a body: its [`Render`] form for [`Format::Text`], its
    /// [`Serialize`] form for [`Format::Json`].
    ///
    /// # Panics
    ///
    /// Panics if `body` fails to render (a `String` sink never does) or to
    /// serialize; both are invariant violations for a derived DTO encoded
    /// into memory, not runtime conditions.
    pub fn encode<T: Serialize + Render>(self, body: &T) -> Encoded {
        match self {
            Self::Text => {
                let mut out = String::new();
                body.render(&mut out).expect("a String sink never fails");
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

/// The text form of a body, for [`Format::Text`].
///
/// [`Serialize`] gives a handler output its JSON form; this gives it the
/// line-oriented one a command writes to stdout. `()` renders nothing and
/// `String` renders itself followed by a newline.
pub trait Render {
    /// Write the text form to `out`.
    ///
    /// # Errors
    ///
    /// Propagates the sink's formatting failure.
    fn render(&self, out: &mut dyn fmt::Write) -> fmt::Result;
}

impl Render for () {
    fn render(&self, _out: &mut dyn fmt::Write) -> fmt::Result {
        Ok(())
    }
}

impl Render for String {
    fn render(&self, out: &mut dyn fmt::Write) -> fmt::Result {
        writeln!(out, "{self}")
    }
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
    broadcast: Broadcast,
    config: Config,
    #[cfg(feature = "docstore")]
    docs: DocumentStore,
    http: HttpRequest,
    identity: Identity,
    model: Model,
    plugins: Plugins,
    publish: Publish,
    state: StateStore,
    #[cfg(feature = "sql")]
    tables: TableStore,
    vault: Vault,
    vcs: Vcs,
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
        handler.call(context, input).instrument(span).await
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
    #[cfg(feature = "http")]
    http: axum::Router<Client<P>>,
    topics: Topics<P>,
    #[cfg(feature = "command")]
    commands: Commands<P>,
}

impl<P: Send + Sync + 'static> Handlers<P> {
    /// Create an empty set of handlers.
    pub fn new() -> Self {
        Self {
            #[cfg(feature = "http")]
            http: axum::Router::new(),
            topics: Topics::new(),
            #[cfg(feature = "command")]
            commands: Commands::new(),
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

    /// Register one handler for one subcommand, decoding its arguments
    /// through [`clap::Args`].
    ///
    /// The input's doc comment is the subcommand's `--help` text; the
    /// output's [`Serialize`] and [`Render`] forms answer `--format json` and
    /// `--format text`.
    ///
    /// # Panics
    ///
    /// Panics when `name` is empty or already registered.
    #[cfg(feature = "command")]
    pub fn command<F, I>(mut self, name: impl Into<String>, handler: F) -> Self
    where
        F: Handler<P, I>,
        F::Output: Serialize + Render,
        F::Error: Into<Failure>,
        I: clap::Args + Send + 'static,
    {
        self.commands = self.commands.route(name, handler);
        self
    }

    /// Register one handler for one [`clap::Command`] subcommand with a
    /// decoder over its [`clap::ArgMatches`].
    ///
    /// # Panics
    ///
    /// Panics when the subcommand's name is empty or already registered.
    #[cfg(feature = "command")]
    pub fn command_with<F, I, D>(mut self, command: clap::Command, handler: F, decode: D) -> Self
    where
        F: Handler<P, I>,
        F::Output: Serialize + Render,
        F::Error: Into<Failure>,
        I: Send + 'static,
        D: Fn(&clap::ArgMatches) -> Result<I, DecodeError> + Send + Sync + 'static,
    {
        self.commands = self.commands.route_with(command, handler, decode);
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

    /// Close the handlers over `client` as the [`Router`] that serves HTTP
    /// requests, dispatches messaging deliveries, and runs argv.
    pub fn with_state(self, client: Client<P>) -> Router<P> {
        Router {
            #[cfg(feature = "http")]
            http: self.http.with_state(client.clone()),
            client,
            topics: self.topics,
            #[cfg(feature = "command")]
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
            #[cfg(feature = "http")]
            http: self.http.clone(),
            topics: self.topics.clone(),
            #[cfg(feature = "command")]
            commands: self.commands.clone(),
        }
    }
}

impl<P: Send + Sync + 'static> fmt::Debug for Handlers<P> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut debug = f.debug_struct("Handlers");
        #[cfg(feature = "http")]
        debug.field("http", &self.http);
        debug.field("topics", &self.topics);
        #[cfg(feature = "command")]
        debug.field("commands", &self.commands);
        debug.finish()
    }
}

#[cfg(feature = "http")]
impl<P: Send + Sync + 'static> From<Handlers<P>> for axum::Router<Client<P>> {
    fn from(handlers: Handlers<P>) -> Self {
        handlers.http
    }
}

/// A finished router backed by a [`Client`], ready to serve HTTP requests,
/// dispatch messaging deliveries, and run argv.
///
/// With the `command` feature the router is also the program clap shows
/// around its subcommands: [`name`](Self::name), [`version`](Self::version),
/// [`about`](Self::about), and [`env_prefix`](Self::env_prefix) set what
/// `--help`, `--version`, and [`Metadata::from_env`] read, and
/// [`hints`](Self::hints) the remedy every failure may carry.
#[must_use = "a router serves nothing until it handles a request or delivery or runs argv"]
pub struct Router<P: Send + Sync + 'static = ()> {
    client: Client<P>,
    #[cfg(feature = "http")]
    http: axum::Router,
    topics: Topics<P>,
    #[cfg(feature = "command")]
    program: Program<P>,
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

    /// Register one handler for one subcommand, decoding its arguments
    /// through [`clap::Args`].
    ///
    /// # Panics
    ///
    /// Panics when `name` is empty or already registered.
    #[cfg(feature = "command")]
    pub fn command<F, I>(mut self, name: impl Into<String>, handler: F) -> Self
    where
        F: Handler<P, I>,
        F::Output: Serialize + Render,
        F::Error: Into<Failure>,
        I: clap::Args + Send + 'static,
    {
        self.program.commands = self.program.commands.route(name, handler);
        self
    }

    /// Register one handler for one [`clap::Command`] subcommand with a
    /// decoder over its [`clap::ArgMatches`].
    ///
    /// # Panics
    ///
    /// Panics when the subcommand's name is empty or already registered.
    #[cfg(feature = "command")]
    pub fn command_with<F, I, D>(mut self, command: clap::Command, handler: F, decode: D) -> Self
    where
        F: Handler<P, I>,
        F::Output: Serialize + Render,
        F::Error: Into<Failure>,
        I: Send + 'static,
        D: Fn(&clap::ArgMatches) -> Result<I, DecodeError> + Send + Sync + 'static,
    {
        self.program.commands = self.program.commands.route_with(command, handler, decode);
        self
    }

    /// Set the program name shown in usage and help text.
    ///
    /// Defaults to `argv[0]` (or `"app"` when `argv` is empty).
    #[cfg(feature = "command")]
    pub fn name(mut self, name: impl Into<String>) -> Self {
        self.program.name = Some(name.into());
        self
    }

    /// Set the version answered for `--version`.
    ///
    /// Defaults to the crate's `CARGO_PKG_VERSION` under [`command::export!`].
    #[cfg(feature = "command")]
    pub fn version(mut self, version: impl Into<String>) -> Self {
        self.program.version = Some(version.into());
        self
    }

    /// Set the description shown at the top of `--help`.
    ///
    /// Defaults to the crate's `CARGO_PKG_DESCRIPTION` under
    /// [`command::export!`].
    #[cfg(feature = "command")]
    pub fn about(mut self, about: impl Into<String>) -> Self {
        self.program.about = Some(about.into());
        self
    }

    /// Set the environment-variable prefix [`run`](Self::run) passes to
    /// [`Metadata::from_env`].
    ///
    /// Defaults to the program name in uppercase with `-` replaced by `_`.
    #[cfg(feature = "command")]
    pub fn env_prefix(mut self, prefix: impl Into<String>) -> Self {
        self.program.env_prefix = Some(prefix.into());
        self
    }

    /// Attach a remedy-hint fn consulted for every command failure that
    /// carries no hint of its own.
    #[cfg(feature = "command")]
    pub fn hints(
        mut self, hint: impl Fn(&crate::Error) -> Option<Cow<'static, str>> + Send + Sync + 'static,
    ) -> Self {
        self.program.hint = Some(Arc::new(hint));
        self
    }

    /// Take every route, topic, and subcommand of `handlers`.
    ///
    /// # Panics
    ///
    /// Panics when a path of `handlers` overlaps an existing route for the same
    /// method, or a topic or subcommand of `handlers` is already registered.
    pub fn merge(mut self, handlers: Handlers<P>) -> Self {
        #[cfg(feature = "http")]
        {
            self.http = self.http.merge(handlers.http.with_state(self.client.clone()));
        }
        self.topics = self.topics.merge(handlers.topics);
        #[cfg(feature = "command")]
        {
            self.program.commands = self.program.commands.merge(handlers.commands);
        }
        self
    }

    /// The exact topics registered on this router, in order.
    pub fn topics(&self) -> impl Iterator<Item = &str> {
        self.topics.topics()
    }

    /// The subcommands registered on this router, in order.
    #[cfg(feature = "command")]
    pub fn commands(&self) -> impl Iterator<Item = &str> {
        self.program.commands.commands()
    }

    /// A shell-completion script for this router's command grammar.
    #[cfg(feature = "command")]
    #[must_use]
    pub fn completions(&self, shell: Shell) -> Response {
        self.program.completions(shell)
    }

    /// Dispatch one delivery by exact topic.
    ///
    /// # Errors
    ///
    /// Returns missing-topic, unhandled-topic, decoding, or handler failures.
    pub async fn handle(&self, delivery: Delivery) -> Result<(), DeliveryError> {
        self.topics.handle(delivery, &self.client).await
    }

    /// Run argv: parse it, read [`Metadata::from_env`], and dispatch the
    /// matched subcommand.
    #[cfg(feature = "command")]
    pub async fn run(&self, argv: impl IntoIterator<Item: Into<OsString>>) -> Response {
        self.program.run(&self.client, argv).await
    }

    /// Run argv with explicit `metadata`.
    #[cfg(feature = "command")]
    pub async fn run_with(
        &self, argv: impl IntoIterator<Item: Into<OsString>>, metadata: &Metadata,
    ) -> Response {
        self.program.run_with(&self.client, argv, metadata).await
    }
}

impl<P: Send + Sync + 'static> Clone for Router<P> {
    fn clone(&self) -> Self {
        Self {
            client: self.client.clone(),
            #[cfg(feature = "http")]
            http: self.http.clone(),
            topics: self.topics.clone(),
            #[cfg(feature = "command")]
            program: self.program.clone(),
        }
    }
}

impl<P: Send + Sync + 'static> fmt::Debug for Router<P> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut debug = f.debug_struct("Router");
        #[cfg(feature = "http")]
        debug.field("http", &self.http);
        debug.field("topics", &self.topics);
        #[cfg(feature = "command")]
        debug.field("program", &self.program);
        debug.finish_non_exhaustive()
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
