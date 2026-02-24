//! # WebSocket Wasm Guest
//!
//! This module demonstrates the WASI WebSocket interface for real-time
//! bidirectional communication. It shows how to:
//! - Connect to a WebSocket socket managed by the host
//! - Create events and send them to connected clients
//! - Optionally target specific groups

#![cfg(target_arch = "wasm32")]

use anyhow::anyhow;
use axum::routing::post;
use axum::{Json, Router};
use omnia_sdk::HttpResult;
use omnia_wasi_websocket::client;
use omnia_wasi_websocket::types::{Client, Error, Event};
use serde_json::{Value, json};
use wasip3::exports::http;
use wasip3::http::types::{ErrorCode, Request, Response};

struct HttpGuest;
wasip3::http::service::export!(HttpGuest);

impl http::handler::Guest for HttpGuest {
    /// Routes HTTP requests to WebSocket management endpoints.
    async fn handle(request: Request) -> Result<Response, ErrorCode> {
        let router = Router::new().route("/", post(send_message));
        omnia_wasi_http::serve(router, request).await
    }
}

/// Sends a message to all connected WebSocket clients.
#[axum::debug_handler]
async fn send_message(message: String) -> HttpResult<Json<Value>> {
    let client =
        Client::connect("default".to_string()).await.map_err(|e| anyhow!("connecting: {e}"))?;
    let event = Event::new(&message.into_bytes());
    client::send(&client, event, None).await.map_err(|e| anyhow!("sending event: {e}"))?;

    Ok(Json(json!({
    "message": "event sent"
    })))
}

struct WebSocket;
omnia_wasi_websocket::export!(WebSocket);

impl omnia_wasi_websocket::handler::Guest for WebSocket {
    async fn handle(event: Event) -> Result<(), Error> {
        println!("received event: {event:?}");

        // let client = Client::connect("default".to_string()).await?;
        // client::send(&client, event, socket_id).await?;

        Ok(())
    }
}
