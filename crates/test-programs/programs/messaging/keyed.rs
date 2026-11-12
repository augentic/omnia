//! A keyed message through the `omnia_sdk` messaging API. The command
//! publishes one keyed `Message` through `Publisher`. The incoming handler
//! routes the delivery through a `Handlers` router and asserts the key arrives
//! as `Delivery::key` beside the publisher's header. It also asserts its
//! context carries the host's deadline, still ahead, rather than the one the
//! delivery arrived with.

#![cfg(target_arch = "wasm32")]

use std::time::SystemTime;

use omnia_sdk::api::messaging::{self, Delivery};
use omnia_sdk::api::{Client, Context, DecodeError, Handlers, Router};
use omnia_sdk::messaging::Message;
use omnia_sdk::{Publisher, WasiProvider};

omnia_sdk::api::command::export!(scenario);
messaging::export!(router);

async fn scenario() {
    let message = Message::new(b"order-7").with_key("customer-7").with_header("tenant", "acme");
    WasiProvider.publish("orders", &message).await.expect("publish");
}

// the decoder hands the handler the whole delivery, key included
fn router(provider: WasiProvider) -> Router<WasiProvider> {
    Handlers::new()
        .topic_with("orders", received, |delivery: &Delivery| {
            Ok::<_, DecodeError>(delivery.clone())
        })
        .with_state(Client::new(provider))
}

async fn received(context: Context<WasiProvider>, delivery: Delivery) -> omnia_sdk::Result<()> {
    assert_eq!(delivery.topic.as_deref(), Some("orders"));
    assert_eq!(&delivery.payload[..], b"order-7");
    assert_eq!(delivery.key.as_deref(), Some("customer-7"));
    let tenant = delivery.metadata.iter().find(|(name, _)| name == "tenant");
    assert_eq!(tenant.map(|(_, value)| value.as_str()), Some("acme"));
    let deadline = context.metadata.deadline.expect("the host stamps every delivery");
    assert!(deadline > SystemTime::now(), "the deadline is the host's, not the delivery's");
    Ok(())
}
