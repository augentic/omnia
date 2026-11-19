//! An `omnia:websocket/incoming-handler` guest: asserts the event it is handed
//! carries the deadline the host set, no metadata and no route, and refuses
//! an empty text. Any other message goes back to its sender in its own kind,
//! and to every other open connection through `broadcast`.

#![cfg(target_arch = "wasm32")]

use std::time::{SystemTime, UNIX_EPOCH};

use omnia_sdk::exports::websocket::incoming_handler::Guest;
use omnia_sdk::websocket::types::{Client, Error, Event, Message};
use omnia_sdk::websocket::{broadcast, connections};

struct WebSocket;
omnia_sdk::exports::websocket::export!(WebSocket);

impl Guest for WebSocket {
    async fn handle(event: Event) -> Result<(), Error> {
        let now = SystemTime::now().duration_since(UNIX_EPOCH).expect("after the epoch");
        let deadline = event.deadline.expect("the host sets every event's deadline");
        assert!(u128::from(deadline) > now.as_millis(), "a deadline ahead, in milliseconds");
        assert!(event.metadata.is_empty(), "no metadata: {:?}", event.metadata);
        assert_eq!(event.route, None);

        if matches!(&event.message, Message::Text(text) if text.is_empty()) {
            return Err(Error::Invalid("an empty message".to_owned()));
        }

        let client = Client::connect("default".to_owned()).await?;
        assert!(
            connections::exists(&client, event.connection.clone()).await?,
            "the sender is open"
        );
        connections::send(&client, vec![event.connection.clone()], event.message.clone()).await?;
        broadcast::send(&client, event.message, vec![event.connection]).await
    }
}
