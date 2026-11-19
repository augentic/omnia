//! A dual-export messaging guest. The command publishes one typed, keyed
//! message through `omnia:messaging/producer`, beside an entry of its own.
//! The `incoming-handler` export asserts the delivery it is handed carries
//! exactly those fields, and the `deadline` the host sets on every delivery,
//! then answers it through `request-reply::reply` on the address the delivery
//! carried, over a client of its own.

#![cfg(target_arch = "wasm32")]

use std::time::{SystemTime, UNIX_EPOCH};

use omnia_wasi_messaging::omnia::incoming_handler::Guest;
use omnia_wasi_messaging::omnia::types::{Client, Delivery, Message, Settle};
use omnia_wasi_messaging::omnia::{producer, request_reply};

omnia_sdk::api::command::export!(scenario);

async fn scenario() {
    let client = Client::connect("default".to_owned()).await.expect("connect");

    let message = Message {
        data: b"order-1".to_vec(),
        content_type: Some("text/plain".to_owned()),
        key: Some("customer-1".to_owned()),
        metadata: vec![("tenant".to_owned(), "acme".to_owned())],
    };
    producer::send(&client, "orders".to_owned(), message).await.expect("send");
}

struct Messaging;
omnia_wasi_messaging::export!(Messaging);

impl Guest for Messaging {
    async fn handle(delivery: Delivery) -> Result<(), Settle> {
        let Delivery {
            topic,
            message,
            reply_to,
            position,
            attempt,
            published_at,
            deadline,
        } = delivery;
        assert_eq!(topic, "orders");
        assert_eq!(message.data, b"order-1");
        assert_eq!(message.content_type.as_deref(), Some("text/plain"));
        assert_eq!(message.key.as_deref(), Some("customer-1"));
        assert_eq!(message.metadata, [("tenant".to_owned(), "acme".to_owned())]);
        assert_eq!((position, attempt, published_at), (None, None, None), "nothing stores it");
        let now = SystemTime::now().duration_since(UNIX_EPOCH).expect("after the epoch");
        let deadline = deadline.expect("the host sets every delivery's deadline");
        assert!(u128::from(deadline) > now.as_millis(), "a deadline ahead, in milliseconds");

        let reply_to = reply_to.expect("the delivery awaits a reply");
        let reply = Message {
            data: b"handled".to_vec(),
            content_type: None,
            key: None,
            metadata: Vec::new(),
        };
        let client = Client::connect("default".to_owned()).await.expect("connect");
        request_reply::reply(&client, reply_to, reply).await.expect("reply");
        Ok(())
    }
}
