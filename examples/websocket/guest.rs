//! # WebSocket Wasm Guest
//!
//! This module demonstrates the WASI WebSocket interface for real-time
//! bidirectional communication. It shows how to:
//! - Connect to a WebSocket service managed by the host
//! - Broadcast a text message to every connected client
//! - Receive the messages connected clients send

#![cfg(target_arch = "wasm32")]

use axum::routing::post;
use axum::{Json, Router};
use omnia_sdk::websocket::types::Error;
use omnia_sdk::websocket::{self, Event, Message};
use serde_json::{Value, json};
use wasip3::exports::http;
use wasip3::http::types::{ErrorCode, Request, Response};

struct HttpGuest;
wasip3::http::service::export!(HttpGuest);

impl http::handler::Guest for HttpGuest {
    async fn handle(request: Request) -> Result<Response, ErrorCode> {
        let router = Router::new().route("/", post(send_message));
        omnia_sdk::http::serve(router, request).await
    }
}

#[axum::debug_handler]
async fn send_message(message: String) -> omnia_sdk::Result<Json<Value>> {
    let client = websocket::default().await?;
    client.broadcast(Message::Text(message), &[]).await?;

    Ok(Json(json!({
    "message": "event sent"
    })))
}

struct WebSocket;
omnia_sdk::exports::websocket::export!(WebSocket);

impl omnia_sdk::exports::websocket::incoming_handler::Guest for WebSocket {
    async fn handle(event: Event) -> Result<(), Error> {
        println!("received event: {event:?}");
        Ok(())
    }
}
