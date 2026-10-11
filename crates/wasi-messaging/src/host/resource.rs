use std::collections::HashMap;
use std::fmt::{self, Debug};
use std::ops::{Deref, DerefMut};
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Context as _;
use futures::future::BoxFuture;
use futures::{Stream, StreamExt as _};
pub use omnia_core::FutureResult;
use tokio::task::{JoinError, JoinSet};

use crate::host::generated::wasi::messaging::types;

/// Stream of messages.
pub type Subscriptions = Pin<Box<dyn Stream<Item = Message> + Send>>;

/// Messaging client trait.
pub trait Client: Debug + Send + Sync + 'static {
    /// Deliver every incoming message to `handler` until the transport gives up.
    ///
    /// The backend owns the delivery loop: how many messages are with
    /// `handler` at once, in what order, and what each outcome means for the
    /// message (committed, redelivered, or fatal). A transport with no
    /// completion semantics hands its stream to [`dispatch`].
    ///
    /// Returning, with `Ok` or `Err`, ends the messaging server: a consumer
    /// never ends on purpose, and the process exits for its orchestrator to
    /// restart it.
    ///
    /// # Errors
    ///
    /// Returns an error if the subscription cannot be opened, the transport
    /// gives up on it, or the backend decides an outcome is fatal.
    fn consume(&self, handler: Arc<dyn Handler>) -> FutureResult<()>;

    /// Send a message to a topic.
    fn send(&self, topic: String, message: Message) -> FutureResult<()>;

    /// Request a response from a topic.
    fn request(
        &self, topic: String, message: Message, options: Option<RequestOptions>,
    ) -> FutureResult<Message>;
}

/// Proxy for a messaging client.
pub type ClientProxy = omnia_core::Proxy<dyn Client>;

/// What a backend delivers each incoming message to.
///
/// The host implements it over the deployment's guests
/// ([`MessagingHandler`](crate::MessagingHandler)); a test scripts one.
pub trait Handler: Send + Sync + 'static {
    /// Handle one message.
    ///
    /// `Ok` when a guest handled the message, or no guest is routed to its
    /// topic. The host logs and counts every failure before returning it, so
    /// a backend only decides what the failure means for the message.
    fn handle(&self, message: Message) -> BoxFuture<'static, Result<(), HandleError>>;
}

/// Why a message was not handled.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub enum HandleError {
    /// No guest ran: the routed guest could not be loaded or instantiated.
    Unavailable(String),
    /// The guest returned `Err`.
    Rejected(types::Error),
    /// The guest trapped.
    Trapped(String),
    /// The guest ran past the deployment's guest timeout.
    TimedOut(Duration),
}

impl fmt::Display for HandleError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unavailable(reason) => write!(f, "no guest ran: {reason}"),
            Self::Rejected(error) => write!(f, "guest returned an error: {error}"),
            Self::Trapped(reason) => write!(f, "guest trapped: {reason}"),
            Self::TimedOut(timeout) => write!(f, "guest ran past {timeout:?}"),
        }
    }
}

impl std::error::Error for HandleError {}

/// Deliver a message stream to `handler`, one task per message, until it ends.
///
/// The delivery loop for a transport with no completion semantics: nothing
/// is held back or redelivered, so a failed outcome is finished with once
/// the handler has logged and counted it. Returns when the stream ends and
/// every message in flight has been handled.
///
/// # Errors
///
/// Returns an error if a handler task panics.
pub async fn dispatch(mut stream: Subscriptions, handler: Arc<dyn Handler>) -> anyhow::Result<()> {
    let mut tasks = JoinSet::new();
    loop {
        tokio::select! {
            message = stream.next() => {
                let Some(message) = message else { break };
                tasks.spawn(handler.handle(message));
            }
            Some(finished) = tasks.join_next() => reap(finished)?,
        }
    }
    while let Some(finished) = tasks.join_next().await {
        reap(finished)?;
    }
    Ok(())
}

// The handler has logged and counted a failed outcome, and this loop holds
// nothing back; only a panicked task is left to report.
fn reap(finished: Result<Result<(), HandleError>, JoinError>) -> anyhow::Result<()> {
    finished.map(drop).context("messaging handler task panicked")
}

/// A message crossing the messaging boundary.
///
/// The host owns message state; backends translate to and from their wire
/// representation at the `Client` seam. Non-exhaustive so a new field is not
/// a breaking change: construct via [`Message::new`] and set fields directly.
#[derive(Clone, Debug, Default)]
#[non_exhaustive]
pub struct Message {
    /// Topic the message is (or was) published to.
    pub topic: String,
    /// Message content.
    pub payload: Vec<u8>,
    /// Headers or metadata associated with the message.
    pub metadata: Option<Metadata>,
    /// Optional reply topic to which a response can be published.
    pub reply: Option<Reply>,
}

impl Message {
    /// Create a message with the given payload.
    #[must_use]
    pub fn new(payload: Vec<u8>) -> Self {
        Self {
            payload,
            ..Self::default()
        }
    }
}

/// Metadata associated with a message.
#[derive(Clone, Debug, Default)]
pub struct Metadata {
    /// The metadata fields.
    pub inner: HashMap<String, String>,
}

impl Metadata {
    /// Create a new empty metadata object.
    #[must_use]
    pub fn new() -> Self {
        Self {
            inner: HashMap::new(),
        }
    }
}

impl Deref for Metadata {
    type Target = HashMap<String, String>;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

impl DerefMut for Metadata {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.inner
    }
}

impl From<Metadata> for types::Metadata {
    fn from(meta: Metadata) -> Self {
        let mut metadata = Self::new();
        for (k, v) in meta.inner {
            metadata.push((k, v));
        }
        metadata
    }
}

impl From<types::Metadata> for Metadata {
    fn from(meta: types::Metadata) -> Self {
        let mut map = HashMap::new();
        for (k, v) in meta {
            map.insert(k, v);
        }
        Self { inner: map }
    }
}

/// Reply information for a message.
#[derive(Clone, Debug, Default)]
pub struct Reply {
    /// The reply topic.
    pub topic: String,
}

/// Options for messaging requests.
#[derive(Clone, Debug, Default)]
pub struct RequestOptions {
    /// Request timeout.
    pub timeout: Option<std::time::Duration>,
    /// Number of expected replies.
    pub expected_replies: Option<u32>,
}
