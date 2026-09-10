//! A dual-export messaging guest. The command publishes one message through
//! `wasi:messaging/producer`; the `incoming-handler` export asserts the
//! message it is handed carries exactly those fields, then answers it
//! through `request-reply::reply` on the reply topic the host attached.

#![cfg(target_arch = "wasm32")]

use omnia_wasi_messaging::incoming_handler::Guest;
use omnia_wasi_messaging::types::{Client, Error, Message};
use omnia_wasi_messaging::{producer, request_reply};

omnia_guest::command!(scenario);

async fn scenario() {
    let client = Client::connect("default".to_owned()).await.expect("connect");

    let message = Message::new(b"order-1");
    message.add_metadata("key", "value");
    producer::send(&client, "orders".to_owned(), message).await.expect("send");
}

struct Messaging;
omnia_wasi_messaging::export!(Messaging with_types_in omnia_wasi_messaging);

impl Guest for Messaging {
    async fn handle(message: Message) -> Result<(), Error> {
        assert_eq!(message.topic().as_deref(), Some("orders"));
        assert_eq!(message.data(), b"order-1");
        assert_eq!(message.content_type(), None, "no content type was set");
        assert_eq!(message.metadata(), Some(vec![("key".to_owned(), "value".to_owned())]));

        request_reply::reply(&message, Message::new(b"handled")).await
    }
}
