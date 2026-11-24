//! `omnia:messaging`, the topics a deployment's broker serves.
//!
//! A guest [`connect`]s a [`Client`] to a broker, or to the [`DEFAULT`]
//! broker with [`default`]. The client [`send`](Client::send)s a
//! [`Message`], or [`request`](Client::request)s a reply to one.
//!
//! A guest receives through its [`Router`](crate::Router). [`export!`] binds
//! the router fn as the guest's `omnia:messaging` handler, and each
//! [`Delivery`] reaches the handler registered for its topic. A guest that
//! settles every delivery itself implements
//! [`exports::messaging`](crate::exports::messaging) instead.
//!
//! The unstable [`consumer`], [`log`], and [`admin`] interfaces are the raw
//! bindings. Every host links them, and answers `unsupported` today.

mod router;

use std::time::Duration;

use serde::Serialize;

pub(crate) use self::router::Topics;
pub use self::router::{Delivery, handle};
pub use crate::bindings::imports::omnia::messaging::*;
use crate::keyvalue::ttl_ms;
pub use crate::messaging::types::{Client, Message};
use crate::{Error, Result};

/// The broker a guest's own [`Client`] connects to.
pub const DEFAULT: &str = "default";

/// Connect to the broker `name`, a binding the deployment declares.
///
/// # Errors
///
/// The host's failure, under its code: `not-found` for a broker the
/// deployment does not declare.
pub async fn connect(name: &str) -> Result<Client> {
    Ok(Client::connect(name.to_owned()).await?)
}

/// Connect to the [`DEFAULT`] broker.
///
/// # Errors
///
/// As [`connect`].
pub async fn default() -> Result<Client> {
    connect(DEFAULT).await
}

impl Client {
    /// Send `message` to `topic`.
    ///
    /// Success means the broker took the message for delivery, not that a
    /// subscriber received it.
    ///
    /// # Errors
    ///
    /// The host's failure, under its code.
    pub async fn send(&self, topic: &str, message: Message) -> Result<()> {
        Ok(producer::send(self, topic.to_owned(), message).await?)
    }

    /// Send `message` to `topic` and return the first reply.
    ///
    /// The call waits `timeout`, or the backend's bound when `None`.
    ///
    /// # Errors
    ///
    /// The host's failure, under its code:
    ///
    /// - `unavailable` when nothing answers in time;
    /// - `invalid` for a zero `timeout`.
    pub async fn request(
        &self, topic: &str, message: Message, timeout: Option<Duration>,
    ) -> Result<Message> {
        let timeout_ms = timeout.map(ttl_ms);
        Ok(request_reply::request(self, topic.to_owned(), message, timeout_ms).await?)
    }

    /// Answer the request whose delivery carried `reply_to`.
    ///
    /// # Errors
    ///
    /// The host's failure, under its code.
    pub async fn reply(&self, reply_to: &str, message: Message) -> Result<()> {
        Ok(request_reply::reply(self, reply_to.to_owned(), message).await?)
    }
}

impl Message {
    /// A message carrying `data`, with no key, content type, or metadata.
    #[must_use]
    pub fn new(data: impl Into<Vec<u8>>) -> Self {
        Self {
            data: data.into(),
            content_type: None,
            key: None,
            metadata: Vec::new(),
        }
    }

    /// A message carrying `value` serialized as JSON, typed `application/json`.
    ///
    /// # Errors
    ///
    /// [`ServerError`](crate::ErrorKind::ServerError) with the code `invalid` when
    /// `value` does not serialize.
    pub fn json<T: Serialize + ?Sized>(value: &T) -> Result<Self> {
        Ok(Self::new(encode(value)?).with_content_type(JSON))
    }

    /// Set the key that orders and partitions the message.
    #[must_use]
    pub fn with_key(mut self, key: impl Into<String>) -> Self {
        self.key = Some(key.into());
        self
    }

    /// Set the data's media type.
    #[must_use]
    pub fn with_content_type(mut self, content_type: impl Into<String>) -> Self {
        self.content_type = Some(content_type.into());
        self
    }

    /// Attach a metadata entry to the message.
    #[must_use]
    pub fn with_metadata(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.metadata.push((name.into(), value.into()));
        self
    }
}

const JSON: &str = "application/json";

fn encode<T: Serialize + ?Sized>(value: &T) -> Result<Vec<u8>> {
    serde_json::to_vec(value)
        .map_err(|err| Error::server_error("invalid", format!("encoding message payload: {err}")))
}

/// Bind a router fn as the guest's `omnia:messaging` export.
///
/// Calls the router fn on each delivery and dispatches the delivery through
/// the [`Router`](crate::Router) it returns, as `handle` does. On a native
/// target it binds nothing, and a private router fn still counts as used.
///
/// ```rust,ignore
/// omnia_sdk::messaging::export!(router);
/// ```
#[doc(inline)]
pub use crate::__export_messaging as export;
