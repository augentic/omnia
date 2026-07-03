//! Default in-memory implementation for wasi-messaging
//!
//! This is a lightweight implementation for development use only.

use std::any::Any;
use std::sync::Arc;

use anyhow::{Result, anyhow};
use futures::FutureExt;
use futures::stream::StreamExt;
use omnia::Backend;
use tokio::sync::broadcast::{self, Receiver, Sender};
use tokio_stream::wrappers::BroadcastStream;
use tracing::instrument;

use crate::host::WasiMessagingCtx;
use crate::host::resource::{
    Client, FutureResult, Message, MessageProxy, Metadata, Reply, RequestOptions, Subscriptions,
};

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

/// Apply `edit` to a clone of the message's inner [`InMemMessage`], returning
/// the updated message; errors if `message` is not an [`InMemMessage`].
fn map_inmem(
    message: &Arc<dyn Message>, edit: impl FnOnce(&mut InMemMessage),
) -> Result<Arc<dyn Message>> {
    let Some(inmem) = message.as_any().downcast_ref::<InMemMessage>() else {
        return Err(wasmtime::Error::msg("invalid message type").into());
    };
    let mut updated = inmem.clone();
    edit(&mut updated);
    Ok(Arc::new(updated) as Arc<dyn Message>)
}

impl WasiMessagingCtx for MessagingDefault {
    fn connect(&self) -> FutureResult<Arc<dyn Client>> {
        tracing::debug!("connecting messaging client");
        let client = self.clone();
        async move { Ok(Arc::new(client) as Arc<dyn Client>) }.boxed()
    }

    fn new_message(&self, data: Vec<u8>) -> Result<Arc<dyn Message>> {
        tracing::debug!("creating new message");
        let message = InMemMessage::from(data);
        Ok(Arc::new(message) as Arc<dyn Message>)
    }

    fn set_content_type(
        &self, message: Arc<dyn Message>, content_type: String,
    ) -> Result<Arc<dyn Message>> {
        tracing::debug!("setting content-type: {content_type}");
        map_inmem(&message, |m| {
            m.metadata.get_or_insert_default().insert("content-type".to_string(), content_type);
        })
    }

    fn set_payload(&self, message: Arc<dyn Message>, data: Vec<u8>) -> Result<Arc<dyn Message>> {
        tracing::debug!("setting payload");
        map_inmem(&message, |m| m.payload = data)
    }

    fn add_metadata(
        &self, message: Arc<dyn Message>, key: String, value: String,
    ) -> Result<Arc<dyn Message>> {
        tracing::debug!("adding metadata: {key} = {value}");
        map_inmem(&message, |m| {
            m.metadata.get_or_insert_default().insert(key, value);
        })
    }

    fn set_metadata(
        &self, message: Arc<dyn Message>, metadata: Metadata,
    ) -> Result<Arc<dyn Message>> {
        tracing::debug!("setting all metadata");
        map_inmem(&message, |m| m.metadata = Some(metadata))
    }

    fn remove_metadata(&self, message: Arc<dyn Message>, key: String) -> Result<Arc<dyn Message>> {
        tracing::debug!("removing metadata: {key}");
        map_inmem(&message, |m| {
            if let Some(md) = m.metadata.as_mut() {
                md.remove(&key);
            }
        })
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

    // Publish/subscribe is covered end-to-end by the seam test
    // (`tests/seam.rs`), which drives the same `MessagingDefault::send` from the
    // guest and observes it on a host `subscribe`. Only the default backend's
    // canned request/reply stub — which no seam exercises — is kept here.
    #[tokio::test]
    async fn request_reply() {
        let backend = <MessagingDefault as Backend>::connect().await.expect("connect");
        let client = WasiMessagingCtx::connect(&backend).await.expect("client");

        let message = backend.new_message(b"ping".to_vec()).expect("new message");
        let reply = client
            .request("topic-c".to_string(), MessageProxy(message), None)
            .await
            .expect("request");

        assert_eq!(reply.payload(), b"ACK");
    }
}
