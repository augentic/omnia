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
        tracing::trace!("connecting messaging client");
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
        tracing::trace!("sending message to topic: {topic}");
        let sender = self.sender.clone();

        async move {
            message.topic = topic;
            // a broadcast send fails only without subscribers, a valid no-op
            let _ = sender.send(message);
            Ok(())
        }
        .boxed()
    }

    fn request(
        &self, topic: String, mut message: Message, _options: Option<RequestOptions>,
    ) -> FutureResult<Message> {
        tracing::trace!("sending request to topic: {}", topic);
        let sender = self.sender.clone();

        async move {
            // a canned reply: publish, then acknowledge without waiting
            message.topic = topic;
            let _ = sender.send(message);

            Ok(Message {
                topic: "response".to_string(),
                payload: b"ACK".to_vec(),
                ..Message::default()
            })
        }
        .boxed()
    }
}
