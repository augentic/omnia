//! Messaging service contract, value types, and default implementation.

use std::collections::HashMap;
use std::ops::{Deref, DerefMut};
use std::time::Duration;

/// Typed messaging failure, mirroring the `wasi:messaging/types` error variant.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    /// The request or operation timed out.
    #[error("timeout")]
    Timeout,
    /// An error occurred with the connection.
    #[error("connection: {0}")]
    Connection(String),
    /// A permission error occurred.
    #[error("permission denied: {0}")]
    PermissionDenied(String),
    /// Any other messaging failure.
    #[error("{0}")]
    Other(String),
}

/// A message crossing the messaging boundary.
///
/// The host owns message state; backends translate to and from their wire
/// representation at the endpoint seam. Non-exhaustive so a new field is not
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
    pub timeout: Option<Duration>,
    /// Number of expected replies.
    pub expected_replies: Option<u32>,
}

#[cfg(not(target_arch = "wasm32"))]
mod host {
    use std::fmt::{self, Debug};
    use std::pin::Pin;
    use std::sync::Arc;
    use std::time::Duration;

    use anyhow::{Context as _, Result};
    use futures::future::BoxFuture;
    use futures::{FutureExt as _, Stream, StreamExt as _};
    use tokio::sync::broadcast::{self, Sender};
    use tokio::task::{JoinError, JoinSet};
    use tokio_stream::wrappers::BroadcastStream;
    use tracing::instrument;

    use super::{Error, Message, RequestOptions};
    use crate::{Connect, FutureResult, NoOptions};

    /// Stream of messages.
    pub type Subscriptions = Pin<Box<dyn Stream<Item = Message> + Send>>;

    /// Service contract for `wasi:messaging` backends.
    pub trait MessagingClient: Debug + Send + Sync + 'static {
        /// Open a messaging endpoint handle.
        fn endpoint(&self) -> FutureResult<Arc<dyn Endpoint>>;
    }

    /// Stateful handle to a messaging transport endpoint.
    pub trait Endpoint: Debug + Send + Sync + 'static {
        /// Deliver every incoming message to `handler` until the transport gives up.
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

    /// Deprecated alias for [`Endpoint`].
    #[deprecated(note = "use `Endpoint`")]
    pub type Client = dyn Endpoint;

    /// What a backend delivers each incoming message to.
    pub trait Handler: Send + Sync + 'static {
        /// Handle one message.
        fn handle(&self, message: Message) -> BoxFuture<'static, Result<(), HandleError>>;
    }

    /// Why a message was not handled.
    #[derive(Clone, Debug)]
    #[non_exhaustive]
    pub enum HandleError {
        /// No guest ran: the routed guest could not be loaded or instantiated.
        Unavailable(String),
        /// The guest returned `Err`.
        Rejected(Error),
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
    /// # Errors
    ///
    /// Returns an error if a handler task panics.
    pub async fn dispatch(mut stream: Subscriptions, handler: Arc<dyn Handler>) -> Result<()> {
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

    fn reap(finished: Result<Result<(), HandleError>, JoinError>) -> Result<()> {
        finished.map(drop).context("messaging handler task panicked")
    }

    /// Default in-process broadcast implementation of [`MessagingClient`].
    #[derive(Clone, Debug)]
    pub struct MessagingDefault {
        sender: Sender<Message>,
    }

    impl MessagingDefault {
        /// Every message published through this broker from now on.
        pub fn subscribe(&self) -> Subscriptions {
            tracing::debug!("subscribing to messages");
            let stream = BroadcastStream::new(self.sender.subscribe());
            Box::pin(stream.filter_map(|res| async move { res.ok() }))
        }
    }

    impl Connect for MessagingDefault {
        type ConnectOptions = NoOptions;

        #[instrument]
        async fn connect_with(options: Self::ConnectOptions) -> Result<Self> {
            let _ = options;
            tracing::debug!("initializing in-memory messaging");
            let (sender, _) = broadcast::channel::<Message>(32);
            Ok(Self { sender })
        }
    }

    impl MessagingClient for MessagingDefault {
        fn endpoint(&self) -> FutureResult<Arc<dyn Endpoint>> {
            tracing::trace!("connecting messaging endpoint");
            let client = self.clone();
            async move { Ok(Arc::new(client) as Arc<dyn Endpoint>) }.boxed()
        }
    }

    impl Endpoint for MessagingDefault {
        fn consume(&self, handler: Arc<dyn Handler>) -> FutureResult<()> {
            dispatch(self.subscribe(), handler).boxed()
        }

        fn send(&self, topic: String, mut message: Message) -> FutureResult<()> {
            tracing::trace!("sending message to topic: {topic}");
            let sender = self.sender.clone();

            async move {
                message.topic = topic;
                let _ = sender.send(message);
                Ok(())
            }
            .boxed()
        }

        fn request(
            &self, topic: String, mut message: Message, _options: Option<RequestOptions>,
        ) -> FutureResult<Message> {
            tracing::trace!("sending request to topic: {}", topic);
            let sender = self.sender.clone();

            async move {
                message.topic = topic;
                let _ = sender.send(message);

                Ok(Message {
                    topic: "response".to_string(),
                    payload: b"ACK".to_vec(),
                    ..Message::default()
                })
            }
            .boxed()
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
#[expect(deprecated, reason = "re-exporting the deprecated alias")]
pub use self::host::Client;
#[cfg(not(target_arch = "wasm32"))]
pub use self::host::{
    Endpoint, HandleError, Handler, MessagingClient, MessagingDefault, Subscriptions, dispatch,
};
