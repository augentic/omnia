//! An `omnia:websocket/handler` guest: asserts the event it is handed
//! carries the frame and origin the host attached, then answers it through
//! `client::send` on a freshly connected client.

#![cfg(target_arch = "wasm32")]

use omnia_wasi_websocket::client;
use omnia_wasi_websocket::handler::Guest;
use omnia_wasi_websocket::types::{Client, Error, Event};

struct WebSocket;
omnia_wasi_websocket::export!(WebSocket);

impl Guest for WebSocket {
    async fn handle(event: Event) -> Result<(), Error> {
        assert_eq!(event.socket_addr().as_deref(), Some("peer-1"));
        assert_eq!(event.data(), b"ping");

        let client = Client::connect("default".to_owned()).await?;
        client::send(&client, Event::new(b"pong"), None).await
    }
}
