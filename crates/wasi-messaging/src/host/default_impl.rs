//! Default in-memory implementation for wasi-messaging
//!
//! This is a lightweight implementation for development use only. Messages
//! fan out over a 32-slot [`tokio::sync::broadcast`] channel: a subscriber
//! that falls more than 32 messages behind silently loses the overwritten
//! messages (the lag error is filtered out of the subscription stream).

use std::sync::Arc;

use anyhow::Result;
use futures::FutureExt;
use futures::stream::StreamExt;
use omnia_core::Backend;
use tokio::sync::broadcast::{self, Sender};
use tokio_stream::wrappers::BroadcastStream;
use tracing::instrument;

use crate::host::WasiMessagingCtx;
use crate::host::resource::{Client, FutureResult, Message, RequestOptions, Subscriptions};

/// Default implementation for `wasi:messaging`.
#[derive(Clone, Debug)]
pub struct MessagingDefault {
    sender: Sender<Message>,
}

impl Backend for MessagingDefault {
    type ConnectOptions = omnia_core::NoOptions;

    #[instrument]
    async fn connect_with(options: Self::ConnectOptions) -> Result<Self> {
        tracing::debug!("initializing in-memory messaging");
        let (sender, _) = broadcast::channel::<Message>(32);
        Ok(Self { sender })
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
        let stream = BroadcastStream::new(self.sender.subscribe());

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
            // A broadcast send only fails when there are no subscribers;
            // publishing into the void is a valid no-op.
            let _ = sender.send(message);
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
            // No subscribers is a valid state for the canned request/reply stub.
            let _ = sender.send(message);

            // Return a simple acknowledgment message
            Ok(Message {
                topic: "response".to_string(),
                payload: b"ACK".to_vec(),
                ..Message::default()
            })
        }
        .boxed()
    }
}
