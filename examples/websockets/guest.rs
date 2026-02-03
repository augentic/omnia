//! # WebSockets Wasm Guest
//!
//! This module demonstrates the WASI WebSockets interface for real-time
//! bidirectional communication. It shows how to:
//! - Access a WebSocket server managed by the host
//! - Query connected peers
//! - Send messages to specific peers
//! - Implement health checks
//!
//! ## Architecture
//!
//! The host manages the WebSocket connections and exposes them to the guest
//! via the WASI WebSockets interface. The guest can:
//! - List connected peers
//! - Send messages to peers
//! - Check server health
//!
//! ## Endpoints
//!
//! - `GET /health`: Check WebSocket server health
//! - `POST /socket`: Send a message to all connected peers

#![cfg(target_arch = "wasm32")]

use anyhow::anyhow;
use axum::routing::{get, post};
use axum::{Json, Router};
use qwasr_sdk::HttpResult;
use qwasr_wasi_websockets::store;
use serde_json::{Value, json};
use wasip3::exports::http::handler::Guest;
use wasip3::http::types::{ErrorCode, Request, Response};

struct HttpGuest;
wasip3::http::service::export!(HttpGuest);

impl Guest for HttpGuest {
    /// Routes HTTP requests to WebSocket management endpoints.
    async fn handle(request: Request) -> Result<Response, ErrorCode> {
        let router =
            Router::new().route("/health", get(get_handler)).route("/socket", post(post_handler));
        qwasr_wasi_http::serve(router, request).await
    }
}

/// Health check endpoint for the WebSocket server.
#[axum::debug_handler]
async fn get_handler() -> HttpResult<Json<Value>> {
    let server = store::get_server().await.map_err(|e| anyhow!("getting websocket server: {e}"))?;

    let message = server.health_check().await.map_err(|e| anyhow!("health check failed: {e}"))?;

    Ok(Json(json!({
        "message": message
    })))
}

/// Sends a message to all connected WebSocket peers.
#[axum::debug_handler]
async fn post_handler(body: String) -> HttpResult<Json<Value>> {
    let server = store::get_server().await.map_err(|e| anyhow!("getting websocket server: {e}"))?;

    let client_peers =
        server.get_peers().await.map_err(|e| anyhow!("getting websocket peers: {e}"))?;

    let recipients: Vec<String> = client_peers.iter().map(|p| p.address.clone()).collect();

    server
        .send_peers(body.to_string(), recipients)
        .await
        .map_err(|e| anyhow!("sending websocket message: {e}"))?;

    Ok(Json(json!({
        "message": "message received"
    })))
}
