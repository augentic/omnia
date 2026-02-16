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
use qwasr_sdk::HttpResult;
use qwasr_wasi_websocket::client;
use qwasr_wasi_websocket::types::{Error, Event, Socket};
use serde_json::{Value, json};
use wasip3::exports::http;
use wasip3::http::types::{ErrorCode, Request, Response};

struct HttpGuest;
wasip3::http::service::export!(HttpGuest);

impl http::handler::Guest for HttpGuest {
    /// Routes HTTP requests to WebSocket management endpoints.
    async fn handle(request: Request) -> Result<Response, ErrorCode> {
        let router = Router::new().route("/socket", post(send_message));
        qwasr_wasi_http::serve(router, request).await
    }
}

/// Sends a message to all connected WebSocket clients.
#[axum::debug_handler]
async fn send_message(message: String) -> HttpResult<Json<Value>> {
    let socket = Socket::connect("default".to_string())
        .await
        .map_err(|e| anyhow!("connecting websocket socket: {e}"))?;

    let event = Event::new(&message.into_bytes());
    client::send(&socket, event, None)
        .await
        .map_err(|e| anyhow!("sending websocket event: {e}"))?;

    Ok(Json(json!({
        "message": "event sent"
    })))
}

struct WebSocketGuest;
qwasr_wasi_websocket::export!(WebSocketGuest);

impl qwasr_wasi_websocket::handler::Guest for WebSocketGuest {
    /// Routes HTTP requests to WebSocket management endpoints.
    async fn handle(event: Event) -> Result<(), Error> {
        let socket = Socket::connect("default".to_string()).await.map_err(|e| Error::from(e))?;

        let event = Event::new(&event.data().to_vec());
        client::send(&socket, event, None).await.map_err(|e| Error::from(e))?;

        Ok(())
    }
}
