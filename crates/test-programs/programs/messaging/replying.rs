//! A messaging guest whose `omnia_sdk` router answers requests. `quotes`
//! replies with its handler's output as JSON, and `echo` with the message its
//! handler builds from the raw payload. The SDK sends each reply to the
//! delivery's reply address through the default broker.

#![cfg(target_arch = "wasm32")]

use omnia_sdk::messaging::{self, Delivery, Message};
use omnia_sdk::{Handlers, Router};
use serde::{Deserialize, Serialize};

messaging::export!(router);

fn router() -> Router {
    Handlers::new()
        .request("quotes", quote)
        .request_with("echo", echo, |delivery: &Delivery| Ok(delivery.payload.to_vec()))
        .into_router()
}

#[derive(Deserialize)]
struct QuoteRequest {
    sku: String,
    quantity: u32,
}

#[derive(Serialize)]
struct Quote {
    sku: String,
    total_cents: u32,
}

async fn quote(request: QuoteRequest) -> omnia_sdk::Result<Quote> {
    Ok(Quote {
        sku: request.sku,
        total_cents: request.quantity * 125,
    })
}

async fn echo(payload: Vec<u8>) -> omnia_sdk::Result<Message> {
    Ok(Message::new(payload).with_content_type("text/plain"))
}
