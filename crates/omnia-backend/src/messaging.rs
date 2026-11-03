//! Messaging service contract, value types, and default implementation.

#[cfg(not(target_arch = "wasm32"))]
mod client;
#[cfg(all(not(target_arch = "wasm32"), feature = "messaging-default"))]
mod default;

use std::collections::HashMap;
use std::ops::{Deref, DerefMut};
use std::time::Duration;

#[cfg(not(target_arch = "wasm32"))]
pub use self::client::{Endpoint, HandleError, Handler, MessagingClient, Subscriptions, dispatch};
#[cfg(all(not(target_arch = "wasm32"), feature = "messaging-default"))]
pub use self::default::MessagingDefault;

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
