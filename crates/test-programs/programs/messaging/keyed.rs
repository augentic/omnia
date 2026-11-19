//! A keyed message through the `omnia_sdk` messaging API. The command sends
//! one keyed message built with the `Message` helpers. The incoming handler
//! routes the delivery through a `Handlers` router and asserts the key arrives
//! as `Delivery::key` beside the message's metadata entry. It also asserts its
//! context carries the host's deadline, still ahead, rather than the
//! `Deadline` entry the delivery arrived with, which reaches it as sent.

#![cfg(target_arch = "wasm32")]

use std::time::SystemTime;

use omnia_sdk::messaging::{self, Delivery};
use omnia_sdk::{Context, Handlers, Router};
use omnia_sdk::messaging::types::{Client, Message};
use omnia_sdk::messaging::{DEFAULT, producer};

omnia_sdk::command::export!(scenario);
messaging::export!(router);

async fn scenario() {
    let client = Client::connect(DEFAULT.to_owned()).await.expect("connect");
    let message = Message::new(b"order-7").with_key("customer-7").with_metadata("tenant", "acme");
    producer::send(&client, "orders".to_owned(), message).await.expect("send");
}

// the decoder hands the handler the whole delivery, key included
fn router() -> Router {
    Handlers::new()
        .topic_with("orders", received, |delivery: &Delivery| Ok(delivery.clone()))
        .into_router()
}

async fn received(context: Context, delivery: Delivery) -> omnia_sdk::Result<()> {
    assert_eq!(delivery.topic.as_deref(), Some("orders"));
    assert_eq!(&delivery.payload[..], b"order-7");
    assert_eq!(delivery.key.as_deref(), Some("customer-7"));
    let entry = |wanted: &str| {
        let found = delivery.metadata.iter().find(|(name, _)| name == wanted);
        found.map(|(_, value)| value.clone())
    };
    assert_eq!(entry("tenant").as_deref(), Some("acme"));
    assert_eq!(entry("Deadline").as_deref(), Some("1"), "the delivery's own entry");
    let deadline = context.metadata.deadline.expect("the host sets every delivery's deadline");
    assert!(deadline > SystemTime::now(), "the deadline is the host's, not the delivery's");
    Ok(())
}
