//! A messaging guest whose `omnia_sdk` router answers requests. `quotes`
//! replies with its handler's output as JSON, and `echo` with the message its
//! handler builds from the raw payload. `slow` replies as `echo` does, after
//! a pause of 300ms. `deadline` replies with the delivery's deadline as text,
//! in milliseconds since the Unix epoch. The SDK sends each reply to the
//! delivery's reply address through the default broker.

#![cfg(target_arch = "wasm32")]

use std::thread;
use std::time::{Duration, UNIX_EPOCH};

use omnia_sdk::messaging::{self, Delivery, Message};
use omnia_sdk::{Handlers, Router};
use serde::{Deserialize, Serialize};

// long enough for the host to stop while the guest waits
const PAUSE: Duration = Duration::from_millis(300);

messaging::export!(router);

fn router() -> Router {
    Handlers::new()
        .request("quotes", quote)
        .request_with("echo", echo, |delivery: &Delivery| Ok(delivery.payload.to_vec()))
        .request_with("slow", slow, |delivery: &Delivery| Ok(delivery.payload.to_vec()))
        .request_with("deadline", echo, |delivery: &Delivery| Ok(deadline_millis(delivery)))
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

async fn slow(payload: Vec<u8>) -> omnia_sdk::Result<Message> {
    thread::sleep(PAUSE);
    echo(payload).await
}

fn deadline_millis(delivery: &Delivery) -> Vec<u8> {
    let deadline = delivery.deadline.expect("the host sets every delivery's deadline");
    let since_epoch = deadline.duration_since(UNIX_EPOCH).expect("after the epoch");
    since_epoch.as_millis().to_string().into_bytes()
}
