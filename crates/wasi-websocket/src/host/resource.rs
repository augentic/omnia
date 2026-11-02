pub use omnia_backend::websocket::{Endpoint, Event, Events};

/// Proxy for a WebSocket server endpoint.
pub type ClientProxy = omnia_core::Proxy<dyn Endpoint>;
