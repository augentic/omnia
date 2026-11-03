//! WebSocket service contract, value types, and default implementation.

#[cfg(not(target_arch = "wasm32"))]
mod client;
#[cfg(all(not(target_arch = "wasm32"), feature = "websocket-default"))]
mod default;

#[cfg(not(target_arch = "wasm32"))]
#[expect(deprecated, reason = "re-exporting the deprecated alias")]
pub use self::client::Client;
#[cfg(not(target_arch = "wasm32"))]
pub use self::client::{Endpoint, Events, WebSocketClient};
#[cfg(all(not(target_arch = "wasm32"), feature = "websocket-default"))]
pub use self::default::{ConnectOptions, WebSocketDefault};

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
