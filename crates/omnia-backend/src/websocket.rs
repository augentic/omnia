//! WebSocket service contract and value types.

#[cfg(not(target_arch = "wasm32"))]
mod client;

#[cfg(not(target_arch = "wasm32"))]
pub use self::client::{Endpoint, Events, WebSocketClient};

/// A WebSocket event crossing the boundary.
///
/// The host owns event state; backends translate to and from their wire
/// representation at the endpoint seam. Non-exhaustive so a new field is
/// not a breaking change: construct via [`Event::new`] and set fields
/// directly.
#[derive(Clone, Debug, Default)]
#[non_exhaustive]
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
