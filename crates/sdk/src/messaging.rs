//! `omnia:messaging`, the topics a deployment's broker serves.
//!
//! A guest connects a [`Client`](types::Client) to a broker, [`DEFAULT`]
//! unless the deployment declares another, and sends a
//! [`Message`](types::Message) through [`producer::send`] or
//! [`request_reply::request`]. It receives through the handler [`export!`]
//! binds.

use serde::Serialize;

pub use crate::bindings::imports::omnia::messaging::{producer, request_reply, types};
use crate::{Error, Result};

/// The broker a guest's own [`Client`](types::Client) connects to.
pub const DEFAULT: &str = "default";

impl types::Message {
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
    /// [`ServerError`](Error::ServerError) with the code `invalid` when
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

#[doc(hidden)]
#[macro_export]
macro_rules! __export_messaging_guest {
    ($ty:ident) => {
        $crate::_export_messaging!($ty);
    };
}

/// Export a type as the guest's `omnia:messaging/incoming-handler`.
///
/// The type implements
/// [`Guest`](crate::exports::messaging::incoming_handler::Guest). A guest
/// that routes by topic binds a router with
/// [`api::messaging::export!`](crate::api::messaging::export!) instead.
#[doc(inline)]
pub use crate::__export_messaging_guest as export;
