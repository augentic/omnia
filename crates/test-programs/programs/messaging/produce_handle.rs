//! A dual-export messaging guest. The command publishes one typed, keyed
//! message through `omnia:messaging/producer`, beside an entry of its own.
//! The `incoming-handler` export asserts the delivery it is handed carries
//! exactly those fields, beside the `deadline` entry the host stamps on every
//! delivery, then answers it through `request-reply::reply` on the address
//! the delivery carried.

#![cfg(target_arch = "wasm32")]

use omnia_wasi_messaging::incoming_handler::Guest;
use omnia_wasi_messaging::types::{Client, Delivery, Error, Message};
use omnia_wasi_messaging::{producer, request_reply};

omnia_sdk::api::command::export!(scenario);

async fn scenario() {
    let client = Client::connect("default".to_owned()).await.expect("connect");

    let message = Message {
        data: b"order-1".to_vec(),
        content_type: Some("text/plain".to_owned()),
        id: None,
        key: Some("customer-1".to_owned()),
        metadata: vec![("tenant".to_owned(), "acme".to_owned())],
    };
    producer::send(&client, "orders".to_owned(), message).await.expect("send");
}

struct Messaging;
omnia_wasi_messaging::export!(Messaging with_types_in omnia_wasi_messaging);

impl Guest for Messaging {
    async fn handle(delivery: Delivery) -> Result<(), Error> {
        let Delivery {
            topic,
            message,
            reply_to,
            position,
            attempt,
            published_at,
        } = delivery;
        assert_eq!(topic, "orders");
        assert_eq!(message.data, b"order-1");
        assert_eq!(message.content_type.as_deref(), Some("text/plain"));
        assert_eq!(message.key.as_deref(), Some("customer-1"));
        assert_eq!(message.id, None);
        let mut metadata = message.metadata;
        metadata.sort();
        let [(deadline, _), (tenant, value)] = &metadata[..] else {
            panic!("expected the publisher's entry and the host's deadline: {metadata:?}");
        };
        assert_eq!([deadline, tenant, value], ["deadline", "tenant", "acme"]);
        assert_eq!((position, attempt, published_at), (None, None, None), "nothing stores it");

        let reply_to = reply_to.expect("the delivery awaits a reply");
        let reply = Message {
            data: b"handled".to_vec(),
            content_type: None,
            id: None,
            key: None,
            metadata: Vec::new(),
        };
        request_reply::reply(reply_to, reply).await
    }
}
