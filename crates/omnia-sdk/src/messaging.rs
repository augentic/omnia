//! Messaging value types.

use std::collections::HashMap;

use bytes::Bytes;
use serde::Serialize;

use crate::capability::{Error, ErrorKind, Result};

// the metadata entry a message key travels in over `wasi:messaging`
#[cfg(all(target_arch = "wasm32", target_os = "wasi"))]
pub(crate) const KEY: &str = "key";

/// A message to publish through a [`Publisher`](crate::Publisher).
///
/// Over `wasi:messaging` the [`key`](Self::key) travels as the `key`
/// metadata entry, and [`Delivery::key`](crate::api::messaging::Delivery::key)
/// reads it back.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct Message {
    /// The message payload.
    pub payload: Bytes,

    /// The key that orders and partitions the message, when it has one.
    pub key: Option<String>,

    /// The payload's media type, when the message names one.
    pub content_type: Option<String>,

    /// The message headers.
    pub headers: HashMap<String, String>,
}

impl Message {
    /// A message carrying `payload`, with no key, content type, or headers.
    #[must_use]
    pub fn new(payload: &[u8]) -> Self {
        Self {
            payload: Bytes::copy_from_slice(payload),
            ..Self::default()
        }
    }

    /// A message carrying `value` serialized as JSON, typed `application/json`.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::Invalid`] when `value` does not serialize.
    pub fn json<T: Serialize + ?Sized>(value: &T) -> Result<Self> {
        let payload = serde_json::to_vec(value).map_err(|err| {
            Error::new(ErrorKind::Invalid, "encoding message payload").with_source(err)
        })?;
        Ok(Self {
            payload: Bytes::from(payload),
            content_type: Some("application/json".to_owned()),
            ..Self::default()
        })
    }

    /// Set the key that orders and partitions the message.
    #[must_use]
    pub fn with_key(mut self, key: impl Into<String>) -> Self {
        self.key = Some(key.into());
        self
    }

    /// Set the payload's media type.
    #[must_use]
    pub fn with_content_type(mut self, content_type: impl Into<String>) -> Self {
        self.content_type = Some(content_type.into());
        self
    }

    /// Attach a header to the message.
    #[must_use]
    pub fn with_header(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.headers.insert(key.into(), value.into());
        self
    }
}
