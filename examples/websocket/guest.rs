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
use omnia_wasi_websocket::broadcast;
use omnia_wasi_websocket::types::{Client, Error, Event, Message};
use serde_json::{Value, json};
use wasip3::exports::http;
use wasip3::http::types::{ErrorCode, Request, Response};

struct HttpGuest;
wasip3::http::service::export!(HttpGuest);

impl http::handler::Guest for HttpGuest {
    async fn handle(request: Request) -> Result<Response, ErrorCode> {
        let router = Router::new().route("/", post(send_message));
        omnia_wasi_http::serve(router, request).await
    }
}

#[axum::debug_handler]
async fn send_message(message: String) -> omnia_sdk::Result<Json<Value>> {
    let client = Client::connect("default".to_string()).await?;
    broadcast::send(&client, Message::Text(message), Vec::new()).await?;

    Ok(Json(json!({
    "message": "event sent"
    })))
}

struct WebSocket;
omnia_wasi_websocket::export!(WebSocket);

impl omnia_wasi_websocket::incoming_handler::Guest for WebSocket {
    async fn handle(event: Event) -> Result<(), Error> {
        println!("received event: {event:?}");
        Ok(())
    }
}
