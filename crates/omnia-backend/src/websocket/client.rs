//! Service contract for `omnia:websocket` backends.

use std::fmt::Debug;
use std::pin::Pin;
use std::sync::Arc;

use futures::Stream;

use super::Event;
use crate::FutureResult;

/// Stream of WebSocket events.
pub type Events = Pin<Box<dyn Stream<Item = Event> + Send>>;

/// Service contract for `omnia:websocket` backends.
pub trait WebSocketClient: Debug + Send + Sync + 'static {
    /// Open a WebSocket endpoint handle.
    fn endpoint(&self) -> FutureResult<Arc<dyn Endpoint>>;
}

/// Stateful handle to a WebSocket endpoint.
pub trait Endpoint: Debug + Send + Sync + 'static {
    /// Subscribe to incoming events from WebSocket clients.
    fn events(&self) -> FutureResult<Events>;

    /// Send an event to connected WebSocket clients, optionally filtered by sockets.
    fn send(&self, event: Event, sockets: Option<Vec<String>>) -> FutureResult<()>;
}
