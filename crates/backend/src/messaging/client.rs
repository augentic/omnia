//! Service contract and dispatch loop for `wasi:messaging` backends.

use std::fmt::Debug;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context as _, Result};
use futures::future::BoxFuture;
use futures::{Stream, StreamExt as _};
use tokio::task::{JoinError, JoinSet};

use super::{Error, Message, RequestOptions};
use crate::FutureResult;

/// Stream of messages.
pub type Subscriptions = Pin<Box<dyn Stream<Item = Message> + Send>>;

/// Service contract for `wasi:messaging` backends.
pub trait MessagingClient: Debug + Send + Sync + 'static {
    /// Open a messaging endpoint handle.
    fn endpoint(&self) -> FutureResult<Arc<dyn Endpoint>>;
}

/// Stateful handle to a messaging transport endpoint.
pub trait Endpoint: Debug + Send + Sync + 'static {
    /// Deliver every incoming message to `handler` until the transport gives up.
    ///
    /// # Errors
    ///
    /// Returns an error if the subscription cannot be opened, the transport
    /// gives up on it, or the backend decides an outcome is fatal.
    fn consume(&self, handler: Arc<dyn Handler>) -> FutureResult<()>;

    /// Send a message to a topic.
    fn send(&self, topic: String, message: Message) -> FutureResult<()>;

    /// Request a response from a topic.
    fn request(
        &self, topic: String, message: Message, options: Option<RequestOptions>,
    ) -> FutureResult<Message>;
}

/// What a backend delivers each incoming message to.
pub trait Handler: Send + Sync + 'static {
    /// Handle one message.
    fn handle(&self, message: Message) -> BoxFuture<'static, Result<(), HandleError>>;
}

/// Why a message was not handled.
#[derive(Clone, Debug, thiserror::Error)]
#[non_exhaustive]
pub enum HandleError {
    /// No guest ran: the routed guest could not be loaded or instantiated.
    #[error("no guest ran: {0}")]
    Unavailable(String),
    /// The guest returned `Err`.
    #[error("guest returned an error: {0}")]
    Rejected(Error),
    /// The guest trapped.
    #[error("guest trapped: {0}")]
    Trapped(String),
    /// The guest ran past the deployment's guest timeout.
    #[error("guest ran past {0:?}")]
    TimedOut(Duration),
}

/// Deliver a message stream to `handler`, one task per message, until it ends.
///
/// # Errors
///
/// Returns an error if a handler task panics.
pub async fn dispatch(mut stream: Subscriptions, handler: Arc<dyn Handler>) -> Result<()> {
    let mut tasks = JoinSet::new();
    loop {
        tokio::select! {
            message = stream.next() => {
                let Some(message) = message else { break };
                tasks.spawn(handler.handle(message));
            }
            Some(finished) = tasks.join_next() => reap(finished)?,
        }
    }
    while let Some(finished) = tasks.join_next().await {
        reap(finished)?;
    }
    Ok(())
}

fn reap(finished: Result<Result<(), HandleError>, JoinError>) -> Result<()> {
    finished.map(drop).context("messaging handler task panicked")
}
