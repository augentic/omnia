//! Message publishing capability.

use std::collections::HashMap;
use std::future::Future;

use anyhow::{Context as _, Result};
use serde::Serialize;

/// A message to be published to a topic.
#[derive(Clone, Debug)]
pub struct Message {
    /// The message payload.
    pub payload: Vec<u8>,
    /// The message headers.
    pub headers: HashMap<String, String>,
}

impl Message {
    /// Create a new message with the specified payload.
    #[must_use]
    pub fn new(payload: &[u8]) -> Self {
        Self {
            payload: payload.to_vec(),
            headers: HashMap::new(),
        }
    }

    /// Serialize `value` as a JSON message payload.
    ///
    /// # Errors
    ///
    /// Returns an error when `value` cannot be serialized to JSON.
    pub fn json<T: Serialize + ?Sized>(value: &T) -> Result<Self> {
        let payload = serde_json::to_vec(value).context("serializing message payload")?;
        Ok(Self {
            payload,
            headers: HashMap::new(),
        })
    }

    /// Attach a header to the message.
    #[must_use]
    pub fn with_header(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.headers.insert(key.into(), value.into());
        self
    }
}

/// Publishes messages to a topic.
#[omnia_guest_macros::capability]
pub trait Publish: Send + Sync {
    /// Publish (send) a message to a topic.
    fn send(&self, topic: &str, message: &Message) -> impl Future<Output = Result<()>> + Send;
}

#[cfg(all(target_arch = "wasm32", target_os = "wasi"))]
impl Publish for crate::WasiProvider {
    async fn send(&self, topic: &str, message: &Message) -> Result<()> {
        use omnia_wasi_messaging::producer;
        use omnia_wasi_messaging::types::{self as wasi, Client};

        let client = Client::connect("host".to_string()).await.context("connecting to broker")?;
        let msg = wasi::Message::new(&message.payload);
        message.headers.iter().for_each(|(k, v)| {
            msg.add_metadata(k, v);
        });
        producer::send(&client, topic.to_string(), msg)
            .await
            .with_context(|| format!("sending message to {topic}"))
    }
}
