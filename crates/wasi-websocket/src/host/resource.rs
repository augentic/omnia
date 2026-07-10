use std::fmt::Debug;
use std::ops::Deref;
use std::pin::Pin;
use std::sync::Arc;

use futures::Stream;
use omnia::FutureResult;

/// Stream of events.
pub type Events = Pin<Box<dyn Stream<Item = Event> + Send>>;

/// Providers implement the [`Client`] trait to allow the host to interact with
/// backend WebSocket resources.
pub trait Client: Debug + Send + Sync + 'static {
    /// Subscribe to incoming events from WebSocket clients.
    fn events(&self) -> FutureResult<Events>;

    /// Send an event to connected WebSocket clients, optionally filtered by sockets.
    fn send(&self, event: Event, sockets: Option<Vec<String>>) -> FutureResult<()>;
}

/// Proxy for a WebSocket server client.
#[derive(Clone, Debug)]
pub struct ClientProxy(pub Arc<dyn Client>);

impl Deref for ClientProxy {
    type Target = Arc<dyn Client>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

/// A WebSocket event crossing the boundary.
///
/// The host owns event state; backends translate to and from their wire
/// representation at the [`Client`] seam.
#[derive(Clone, Debug, Default)]
pub struct Event {
    /// The socket address this event was received from, when known.
    pub socket_addr: Option<String>,
    /// The event data.
    pub data: Vec<u8>,
    /// The route key used to select a guest, when the event carries one.
    ///
    /// `None` fans the event into the trigger's catch-all guest (the sole
    /// websocket exporter), preserving single-guest behaviour.
    pub route: Option<String>,
}

impl Event {
    /// Create an event with the given payload.
    #[must_use]
    pub fn new(data: Vec<u8>) -> Self {
        Self {
            data,
            ..Self::default()
        }
    }
}
