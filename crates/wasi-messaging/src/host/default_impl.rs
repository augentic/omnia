//! In-process broadcast default implementation of [`MessagingClient`].

use std::fmt::Debug;
use std::sync::Arc;

use anyhow::Result;
use futures::{FutureExt as _, StreamExt as _};
use omnia_backend::messaging::{
    Endpoint, Handler, Message, MessagingClient, RequestOptions, Subscriptions, dispatch,
};
use omnia_core::{Connect, FutureResult, NoOptions};
use tokio::sync::broadcast::{self, Sender};
use tokio_stream::wrappers::BroadcastStream;
use tracing::instrument;

/// Default in-process broadcast implementation of [`MessagingClient`].
#[derive(Clone, Debug)]
pub struct MessagingDefault {
    sender: Sender<Message>,
}

impl MessagingDefault {
    /// Every message published through this broker from now on.
    pub fn subscribe(&self) -> Subscriptions {
        tracing::debug!("subscribing to messages");
        let stream = BroadcastStream::new(self.sender.subscribe());
        Box::pin(stream.filter_map(|res| async move { res.ok() }))
    }
}

impl Connect for MessagingDefault {
    type ConnectOptions = NoOptions;

    #[instrument]
    async fn connect_with(options: Self::ConnectOptions) -> Result<Self> {
        let _ = options;
        tracing::debug!("initializing in-memory messaging");
        let (sender, _) = broadcast::channel::<Message>(32);
        Ok(Self { sender })
    }
}

impl MessagingClient for MessagingDefault {
    fn endpoint(&self) -> FutureResult<Arc<dyn Endpoint>> {
        tracing::trace!("connecting messaging endpoint");
        let client = self.clone();
        async move { Ok(Arc::new(client) as Arc<dyn Endpoint>) }.boxed()
    }
}

impl Endpoint for MessagingDefault {
    fn consume(&self, handler: Arc<dyn Handler>) -> FutureResult<()> {
        dispatch(self.subscribe(), handler).boxed()
    }

    fn send(&self, topic: String, mut message: Message) -> FutureResult<()> {
        tracing::trace!("sending message to topic: {topic}");
        let sender = self.sender.clone();

        async move {
            message.topic = topic;
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
            message.topic = topic;
            let _ = sender.send(message);

            let mut resp = Message::new(b"ACK".to_vec());
            resp.topic = "response".to_string();
            Ok(resp)
        }
        .boxed()
    }
}
