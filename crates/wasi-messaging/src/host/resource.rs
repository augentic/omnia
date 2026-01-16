use std::any::Any;
use std::collections::HashMap;
use std::fmt::Debug;
use std::ops::{Deref, DerefMut};
use std::pin::Pin;
use std::sync::Arc;

use futures::Stream;
pub use qwasr::FutureResult;
use serde::{Deserialize, Serialize};

use crate::host::generated::wasi::messaging::types;
/// Stream of message proxies.
pub type Subscriptions = Pin<Box<dyn Stream<Item = MessageProxy> + Send>>;

/// Messaging client trait.
#[allow(unused_variables)]
pub trait Client: Debug + Send + Sync + 'static {
    /// Subscribe to messages.
    fn subscribe(&self) -> FutureResult<Subscriptions>;

    /// Send a message to a topic.
    fn send(&self, topic: String, message: MessageProxy) -> FutureResult<()>;

    /// Request a response from a topic.
    fn request(
        &self, topic: String, message: MessageProxy, options: Option<RequestOptions>,
    ) -> FutureResult<MessageProxy>;
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

/// Providers implement the [`Message`] trait to allow the host to interact with
/// different backend messaging systems.
pub trait Message: Debug + Send + Sync + 'static {
    /// Topic the message is published to.
    fn topic(&self) -> String;

    /// Message content.
    fn payload(&self) -> Vec<u8>;

    /// Headers or metadata associated with the message.
    fn metadata(&self) -> Option<Metadata>;

    /// Optional message description.
    fn description(&self) -> Option<String>;

    /// Number of bytes in the payload.
    fn length(&self) -> usize;

    /// Optional reply topic to which a response can be published.
    fn reply(&self) -> Option<Reply>;

    /// For downcasting support.
    fn as_any(&self) -> &dyn Any;
}

/// Proxy for a message.
#[derive(Clone, Debug)]
pub struct MessageProxy(pub Arc<dyn Message>);

impl Deref for MessageProxy {
    type Target = Arc<dyn Message>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for MessageProxy {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
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
