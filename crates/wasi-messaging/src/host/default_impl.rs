//! Default in-memory implementation for wasi-messaging
//!
//! This is a lightweight implementation for development use only.

use std::any::Any;
use std::sync::Arc;

use anyhow::{Result, anyhow};
use futures::FutureExt;
use futures::stream::StreamExt;
use tokio::sync::broadcast::{self, Receiver, Sender};
use tokio_stream::wrappers::BroadcastStream;
use tracing::instrument;
use yetti::Backend;

use crate::host::WasiMessagingCtx;
use crate::host::resource::{
    Client, FutureResult, Message, MessageProxy, Metadata, Reply, RequestOptions, Subscriptions,
};

#[derive(Debug, Clone, Default)]
pub struct ConnectOptions;

impl yetti::FromEnv for ConnectOptions {
    fn from_env() -> Result<Self> {
        Ok(Self)
    }
}

#[derive(Debug)]
pub struct MessagingDefault {
    sender: Sender<MessageProxy>,
    receiver: Receiver<MessageProxy>,
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
        let (sender, receiver) = broadcast::channel::<MessageProxy>(32);
        Ok(Self { sender, receiver })
    }
}

impl WasiMessagingCtx for MessagingDefault {
    fn connect(&self) -> FutureResult<Arc<dyn Client>> {
        tracing::debug!("connecting messaging client");
        let client = self.clone();
        async move { Ok(Arc::new(client) as Arc<dyn Client>) }.boxed()
    }

    fn new_message(&self, data: Vec<u8>) -> anyhow::Result<Arc<dyn Message>> {
        tracing::debug!("creating new message");
        let message = InMemMessage::from(data);
        Ok(Arc::new(message) as Arc<dyn Message>)
    }

    fn set_content_type(
        &self, message: Arc<dyn Message>, content_type: String,
    ) -> anyhow::Result<Arc<dyn Message>> {
        tracing::debug!("setting content-type: {}", content_type);

        let Some(inmem) = message.as_any().downcast_ref::<InMemMessage>() else {
            anyhow::bail!("invalid message type");
        };

        let mut updated = inmem.clone();
        let mut metadata = updated.metadata.unwrap_or_default();
        metadata.insert("content-type".to_string(), content_type);
        updated.metadata = Some(metadata);

        Ok(Arc::new(updated) as Arc<dyn Message>)
    }

    fn set_payload(
        &self, message: Arc<dyn Message>, data: Vec<u8>,
    ) -> anyhow::Result<Arc<dyn Message>> {
        tracing::debug!("setting payload");

        let Some(inmem) = message.as_any().downcast_ref::<InMemMessage>() else {
            anyhow::bail!("invalid message type");
        };

        let mut updated = inmem.clone();
        updated.payload = data;

        Ok(Arc::new(updated) as Arc<dyn Message>)
    }

    fn add_metadata(
        &self, message: Arc<dyn Message>, key: String, value: String,
    ) -> anyhow::Result<Arc<dyn Message>> {
        tracing::debug!("adding metadata: {key} = {value}");

        let Some(inmem) = message.as_any().downcast_ref::<InMemMessage>() else {
            anyhow::bail!("invalid message type");
        };

        let mut updated = inmem.clone();
        let mut metadata = updated.metadata.unwrap_or_default();
        metadata.insert(key, value);
        updated.metadata = Some(metadata);

        Ok(Arc::new(updated) as Arc<dyn Message>)
    }

    fn set_metadata(
        &self, message: Arc<dyn Message>, metadata: Metadata,
    ) -> anyhow::Result<Arc<dyn Message>> {
        tracing::debug!("setting all metadata");

        let Some(inmem) = message.as_any().downcast_ref::<InMemMessage>() else {
            anyhow::bail!("invalid message type");
        };

        let mut updated = inmem.clone();
        updated.metadata = Some(metadata);

        Ok(Arc::new(updated) as Arc<dyn Message>)
    }

    fn remove_metadata(
        &self, message: Arc<dyn Message>, key: String,
    ) -> anyhow::Result<Arc<dyn Message>> {
        tracing::debug!("removing metadata: {}", key);

        let Some(inmem) = message.as_any().downcast_ref::<InMemMessage>() else {
            anyhow::bail!("invalid message type");
        };

        let mut updated = inmem.clone();
        if let Some(ref mut metadata) = updated.metadata {
            metadata.remove(&key);
        }

        Ok(Arc::new(updated) as Arc<dyn Message>)
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

    fn send(&self, topic: String, message: MessageProxy) -> FutureResult<()> {
        tracing::debug!("sending message to topic: {topic}");
        let sender = self.sender.clone();

        async move {
            let Some(inmem) = message.as_any().downcast_ref::<InMemMessage>() else {
                anyhow::bail!("invalid message type");
            };

            let mut updated = inmem.clone();
            updated.topic.clone_from(&topic);
            let msg_proxy = MessageProxy(Arc::new(updated) as Arc<dyn Message>);

            sender.send(msg_proxy).map_err(|e| anyhow!("send error: {e}"))?;

            Ok(())
        }
        .boxed()
    }

    fn request(
        &self, topic: String, message: MessageProxy, _options: Option<RequestOptions>,
    ) -> FutureResult<MessageProxy> {
        tracing::debug!("sending request to topic: {}", topic);
        let sender = self.sender.clone();

        async move {
            // In a real implementation, this would send a request and wait for a response
            // For the default impl, we'll just create a simple response
            let Some(inmem) = message.as_any().downcast_ref::<InMemMessage>() else {
                anyhow::bail!("invalid message type");
            };

            let mut updated = inmem.clone();
            updated.topic.clone_from(&topic);

            let msg_proxy = MessageProxy(Arc::new(updated) as Arc<dyn Message>);
            sender.send(msg_proxy).map_err(|e| anyhow!("send error: {e}"))?;

            // Return a simple acknowledgment message
            let response = InMemMessage {
                topic: "response".to_string(),
                payload: b"ACK".to_vec(),
                metadata: None,
                description: Some("default response".to_string()),
                reply: None,
            };

            Ok(MessageProxy(Arc::new(response)))
        }
        .boxed()
    }
}

#[derive(Debug, Clone, Default)]
struct InMemMessage {
    topic: String,
    payload: Vec<u8>,
    metadata: Option<Metadata>,
    description: Option<String>,
    reply: Option<Reply>,
}

impl From<Vec<u8>> for InMemMessage {
    fn from(data: Vec<u8>) -> Self {
        Self {
            topic: String::new(),
            payload: data,
            metadata: None,
            description: None,
            reply: None,
        }
    }
}

impl Message for InMemMessage {
    fn topic(&self) -> String {
        self.topic.clone()
    }

    fn payload(&self) -> Vec<u8> {
        self.payload.clone()
    }

    fn metadata(&self) -> Option<Metadata> {
        self.metadata.clone()
    }

    fn description(&self) -> Option<String> {
        self.description.clone()
    }

    fn length(&self) -> usize {
        self.payload.len()
    }

    fn reply(&self) -> Option<Reply> {
        self.reply.clone()
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn messaging() {
        let ctx = MessagingDefault::connect_with(ConnectOptions).await.expect("connect");

        // Test connect
        let client = ctx.connect().await.expect("connect client");

        // Test new_message
        let message = ctx.new_message(b"test payload".to_vec()).expect("new message");
        assert_eq!(message.payload(), b"test payload".to_vec());
        assert_eq!(message.length(), 12);

        // Test set_content_type
        let message = ctx
            .set_content_type(message, "application/json".to_string())
            .expect("set content type");
        assert!(message.metadata().is_some());

        // Test add_metadata
        let message = ctx
            .add_metadata(message, "custom-key".to_string(), "custom-value".to_string())
            .expect("add metadata");
        let metadata = message.metadata().expect("metadata");
        assert_eq!(metadata.get("custom-key"), Some(&"custom-value".to_string()));

        // Test send
        client.send("test-topic".to_string(), MessageProxy(message)).await.expect("send");
    }
}
