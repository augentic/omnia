//! Hand-bound `omnia:messaging` and `omnia:websocket` incoming handlers, each
//! opening one span named for its export. Neither handler opens a telemetry
//! scope, so a span reaches the host only through its binding's.

#![cfg(target_arch = "wasm32")]

use omnia_sdk::messaging::types::{Delivery, Settle};
use omnia_sdk::websocket::types::{Error, Event};

struct Messaging;
omnia_sdk::exports::messaging::export!(Messaging);

// ERROR-level throughout: each span must pass the guest's `EnvFilter` at the
// run's default level.
impl omnia_sdk::exports::messaging::incoming_handler::Guest for Messaging {
    async fn handle(_delivery: Delivery) -> Result<(), Settle> {
        tracing::error_span!("messaging").in_scope(|| ());
        Ok(())
    }
}

struct WebSocket;
omnia_sdk::exports::websocket::export!(WebSocket);

impl omnia_sdk::exports::websocket::incoming_handler::Guest for WebSocket {
    async fn handle(_event: Event) -> Result<(), Error> {
        tracing::error_span!("websocket").in_scope(|| ());
        Ok(())
    }
}
