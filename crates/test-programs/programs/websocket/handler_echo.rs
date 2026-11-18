//! An `omnia:websocket/incoming-handler` guest: asserts the event it is handed
//! carries the deadline the host stamped and no route, and refuses an empty
//! text. Any other message goes back to its sender in its own kind, and to
//! every other open connection through `broadcast`.

#![cfg(target_arch = "wasm32")]

use omnia_wasi_websocket::incoming_handler::Guest;
use omnia_wasi_websocket::types::{Client, Error, Event, Message};
use omnia_wasi_websocket::{broadcast, connections};

struct WebSocket;
omnia_wasi_websocket::export!(WebSocket);

impl Guest for WebSocket {
    async fn handle(event: Event) -> Result<(), Error> {
        let deadline = event.metadata.iter().find(|(name, _)| name == "deadline");
        assert!(
            deadline.is_some_and(|(_, millis)| millis.parse::<u64>().is_ok()),
            "a deadline in milliseconds: {:?}",
            event.metadata
        );
        assert_eq!(event.route, None);

        if matches!(&event.message, Message::Text(text) if text.is_empty()) {
            return Err(Error::Invalid("an empty message".to_owned()));
        }

        let client = Client::connect("default".to_owned()).await?;
        assert!(
            connections::exists(&client, event.connection.clone()).await?,
            "the sender is open"
        );
        connections::send(&client, event.connection.clone(), event.message.clone()).await?;
        broadcast::send(&client, event.message, vec![event.connection]).await
    }
}
