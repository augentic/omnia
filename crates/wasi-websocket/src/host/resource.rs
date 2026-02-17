use std::any::Any;
use std::fmt::Debug;
use std::ops::{Deref, DerefMut};
use std::pin::Pin;
use std::sync::Arc;

use futures::Stream;
use qwasr::FutureResult;

/// Stream of event proxies.
pub type Subscriptions = Pin<Box<dyn Stream<Item = EventProxy> + Send>>;

/// Providers implement the [`Socket`] trait to allow the host to interact with
/// backend WebSocket resources.
#[allow(unused_variables)]
pub trait Socket: Debug + Send + Sync + 'static {
    /// Subscribe to incoming events from WebSocket clients.
    fn subscribe(&self) -> FutureResult<Subscriptions>;

    /// Send an event to connected WebSocket clients, optionally filtered by groups.
    fn send(&self, event: EventProxy, groups: Option<Vec<String>>) -> FutureResult<()>;
}

/// Proxy for a WebSocket socket.
#[derive(Clone, Debug)]
pub struct SocketProxy(pub Arc<dyn Socket>);

impl Deref for SocketProxy {
    type Target = Arc<dyn Socket>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

/// Providers implement the [`Event`] trait to represent WebSocket events.
pub trait Event: Debug + Send + Sync + 'static {
    /// The group this event was received on, if any.
    fn group(&self) -> Option<String>;

    /// The event data.
    fn data(&self) -> Vec<u8>;

    /// For downcasting support.
    fn as_any(&self) -> &dyn Any;
}

/// Proxy for a WebSocket event.
#[derive(Clone, Debug)]
pub struct EventProxy(pub Arc<dyn Event>);

impl Deref for EventProxy {
    type Target = Arc<dyn Event>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for EventProxy {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}
