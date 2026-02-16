//! # WebSocket Wasm Guest
//!
//! This module demonstrates the WASI WebSocket interface for real-time
//! bidirectional communication. It shows how to:
//! - Access a WebSocket server managed by the host
//! - Query connected peers
//! - Send messages to specific peers

#![cfg(target_arch = "wasm32")]

use anyhow::anyhow;
use axum::routing::{get, post};
use axum::{Json, Router};
use qwasr_sdk::HttpResult;
use qwasr_wasi_websocket::store;
use serde_json::{Value, json};
use wasip3::exports::http::handler::Guest;
use wasip3::http::types::{ErrorCode, Request, Response};

struct HttpGuest;
wasip3::http::service::export!(HttpGuest);

impl Guest for HttpGuest {
    /// Routes HTTP requests to WebSocket management endpoints.
    async fn handle(request: Request) -> Result<Response, ErrorCode> {
        let router = Router::new().route("/socket", post(send_message));
        qwasr_wasi_http::serve(router, request).await
    }
}

/// Sends a message to all connected WebSocket peers.
#[axum::debug_handler]
async fn send_message(message: String) -> HttpResult<Json<Value>> {
    let server = store::get_server().await.map_err(|e| anyhow!("getting websocket server: {e}"))?;

    let client_peers =
        server.get_peers().await.map_err(|e| anyhow!("getting websocket peers: {e}"))?;
    let recipients: Vec<String> = client_peers.iter().map(|p| p.address.clone()).collect();

    server
        .send_peers(message, recipients)
        .await
        .map_err(|e| anyhow!("sending websocket message: {e}"))?;

    Ok(Json(json!({
        "message": "message received"
    })))
}
