use std::collections::HashMap;
use std::fmt::Debug;
use std::ops::{Deref, DerefMut};
use std::pin::Pin;
use std::sync::Arc;

use futures::Stream;
pub use omnia::FutureResult;
use serde::{Deserialize, Serialize};

use crate::host::generated::wasi::messaging::types;
/// Stream of messages.
pub type Subscriptions = Pin<Box<dyn Stream<Item = Message> + Send>>;

/// Messaging client trait.
pub trait Client: Debug + Send + Sync + 'static {
    /// Subscribe to messages.
    fn subscribe(&self) -> FutureResult<Subscriptions>;

    /// Send a message to a topic.
    fn send(&self, topic: String, message: Message) -> FutureResult<()>;

    /// Request a response from a topic.
    fn request(
        &self, topic: String, message: Message, options: Option<RequestOptions>,
    ) -> FutureResult<Message>;
}

/// Proxy for a messaging client.
#[derive(Clone, Debug)]
pub struct ClientProxy(pub Arc<dyn Client>);

impl Deref for ClientProxy {
    type Target = Arc<dyn Client>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

/// A message crossing the messaging boundary.
///
/// The host owns message state; backends translate to and from their wire
/// representation at the `Client` seam.
#[derive(Clone, Debug, Default)]
pub struct Message {
    /// Topic the message is (or was) published to.
    pub topic: String,
    /// Message content.
    pub payload: Vec<u8>,
    /// Headers or metadata associated with the message.
    pub metadata: Option<Metadata>,
    /// Optional message description.
    pub description: Option<String>,
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

    /// Message content.
    #[must_use]
    pub fn payload(&self) -> &[u8] {
        &self.payload
    }
}

/// Metadata associated with a message.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
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
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct Reply {
    /// The client name.
    pub client_name: String,
    /// The reply topic.
    pub topic: String,
}

/// Options for messaging requests.
#[derive(Default, Clone)]
pub struct RequestOptions {
    /// Request timeout.
    pub timeout: Option<std::time::Duration>,
    /// Number of expected replies.
    pub expected_replies: Option<u32>,
}
