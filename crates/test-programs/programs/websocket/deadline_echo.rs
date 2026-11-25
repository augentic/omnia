//! An `omnia:websocket/incoming-handler` guest that sends each event's
//! deadline back to its sender, as the text of its milliseconds since the
//! Unix epoch.

#![cfg(target_arch = "wasm32")]

use omnia_sdk::exports::websocket::incoming_handler::Guest;
use omnia_sdk::websocket::connections;
use omnia_sdk::websocket::types::{Client, Error, Event, Message};

struct WebSocket;
omnia_sdk::exports::websocket::export!(WebSocket);

impl Guest for WebSocket {
    async fn handle(event: Event) -> Result<(), Error> {
        let deadline = event.deadline.expect("the host sets every event's deadline");
        let client = Client::connect("default".to_owned()).await?;
        connections::send(&client, vec![event.connection], Message::Text(deadline.to_string()))
            .await
    }
}
