//! Default in-memory implementation for wasi-messaging
//!
//! This is a lightweight implementation for development use only.

use std::sync::Arc;

use anyhow::{Result, anyhow};
use futures::FutureExt;
use futures::stream::StreamExt;
use omnia::Backend;
use tokio::sync::broadcast::{self, Receiver, Sender};
use tokio_stream::wrappers::BroadcastStream;
use tracing::instrument;

use crate::host::WasiMessagingCtx;
use crate::host::resource::{Client, FutureResult, Message, RequestOptions, Subscriptions};

/// Options used to connect to the messaging system.
#[derive(Debug, Clone, Default)]
pub struct ConnectOptions;

impl omnia::FromEnv for ConnectOptions {
    fn from_env() -> Result<Self> {
        Ok(Self)
    }
}

/// Default implementation for `wasi:messaging`.
#[derive(Debug)]
pub struct MessagingDefault {
    sender: Sender<Message>,
    receiver: Receiver<Message>,
}

impl Clone for MessagingDefault {
    fn clone(&self) -> Self {
        Self {
            sender: self.sender.clone(),
            receiver: self.sender.subscribe(),
        }
    }
}

impl Backend for MessagingDefault {
    type ConnectOptions = ConnectOptions;

    #[instrument]
    async fn connect_with(options: Self::ConnectOptions) -> Result<Self> {
        tracing::debug!("initializing in-memory messaging");
        let (sender, receiver) = broadcast::channel::<Message>(32);
        Ok(Self { sender, receiver })
    }
}

impl WasiMessagingCtx for MessagingDefault {
    fn connect(&self) -> FutureResult<Arc<dyn Client>> {
        tracing::debug!("connecting messaging client");
        let client = self.clone();
        async move { Ok(Arc::new(client) as Arc<dyn Client>) }.boxed()
    }
}

impl Client for MessagingDefault {
    fn subscribe(&self) -> FutureResult<Subscriptions> {
        tracing::debug!("subscribing to messages");
        let stream = BroadcastStream::new(self.receiver.resubscribe());

        async move {
            let stream = stream.filter_map(|res| async move { res.ok() });
            Ok(Box::pin(stream) as Subscriptions)
        }
        .boxed()
    }

    fn send(&self, topic: String, mut message: Message) -> FutureResult<()> {
        tracing::debug!("sending message to topic: {topic}");
        let sender = self.sender.clone();

        async move {
            message.topic = topic;
            sender.send(message).map_err(|e| anyhow!("send error: {e}"))?;
            Ok(())
        }
        .boxed()
    }

    fn request(
        &self, topic: String, mut message: Message, _options: Option<RequestOptions>,
    ) -> FutureResult<Message> {
        tracing::debug!("sending request to topic: {}", topic);
        let sender = self.sender.clone();

        async move {
            // In a real implementation, this would send a request and wait for a response
            // For the default impl, we'll just create a simple response
            message.topic = topic;
            sender.send(message).map_err(|e| anyhow!("send error: {e}"))?;

            // Return a simple acknowledgment message
            Ok(Message {
                topic: "response".to_string(),
                payload: b"ACK".to_vec(),
                description: Some("default response".to_string()),
                ..Message::default()
            })
        }
        .boxed()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Publish/subscribe is covered end-to-end by the seam test
    // (`tests/seam.rs`), which drives the same `MessagingDefault::send` from the
    // guest and observes it on a host `subscribe`. Only the default backend's
    // canned request/reply stub — which no seam exercises — is kept here.
    #[tokio::test]
    async fn request_reply() {
        let backend = <MessagingDefault as Backend>::connect().await.expect("connect");
        let client = WasiMessagingCtx::connect(&backend).await.expect("client");

        let reply = client
            .request("topic-c".to_string(), Message::new(b"ping".to_vec()), None)
            .await
            .expect("request");

        assert_eq!(reply.payload(), b"ACK");
    }
}
