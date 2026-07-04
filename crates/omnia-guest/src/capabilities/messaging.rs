//! Message publishing capability.

use std::collections::HashMap;
use std::future::Future;

use anyhow::Result;

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
}

/// Publishes messages to a topic.
pub trait Publish: Send + Sync {
    /// Publish (send) a message to a topic.
    #[cfg(not(target_arch = "wasm32"))]
    fn send(&self, topic: &str, message: &Message) -> impl Future<Output = Result<()>> + Send;

    /// Publish (send) a message to a topic.
    #[cfg(target_arch = "wasm32")]
    fn send(&self, topic: &str, message: &Message) -> impl Future<Output = Result<()>> + Send {
        use anyhow::Context;
        use omnia_wasi_messaging::producer;
        use omnia_wasi_messaging::types::{self as wasi, Client};

        async move {
            let client =
                Client::connect("host".to_string()).await.context("connecting to broker")?;
            let msg = wasi::Message::new(&message.payload);
            message.headers.iter().for_each(|(k, v)| {
                msg.add_metadata(k, v);
            });
            producer::send(&client, topic.to_string(), msg)
                .await
                .with_context(|| format!("sending message to {topic}"))
        }
    }
}
