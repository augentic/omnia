//! # HTTP Server Wasm Guest
//!
//! This module demonstrates the basic WASI HTTP handler pattern. It shows how to:
//! - Implement the WASI HTTP `Guest` trait
//! - Use Axum for routing within a WebAssembly guest
//! - Handle JSON request/response bodies
//! - Integrate OpenTelemetry tracing

#![cfg(target_arch = "wasm32")]

use axum::routing::{get, post};
use axum::{Json, Router};
use qwasr_sdk::HttpResult;
use serde_json::{Value, json};
use tracing::Level;
use wasip3::exports::http::handler::Guest;
use wasip3::http::types::{ErrorCode, Request, Response};

struct HttpGuest;
wasip3::http::service::export!(HttpGuest);

impl Guest for HttpGuest {
    /// Routes incoming HTTP requests to handlers.
    #[qwasr_wasi_otel::instrument(name = "http_guest_handle", level = Level::DEBUG)]
    async fn handle(request: Request) -> Result<Response, ErrorCode> {
        let router = Router::new().route("/", get(echo_get)).route("/", post(echo_post));
        qwasr_wasi_http::serve(router, request).await
    }
}

/// GET request handler.
#[qwasr_wasi_otel::instrument]
async fn echo_get(Json(body): Json<Value>) -> HttpResult<Json<Value>> {
    Ok(Json(json!({
        "message": "Hello from echo_get!",
        "request": body
    })))
}

/// POST request handler.
#[qwasr_wasi_otel::instrument]
async fn echo_post(Json(body): Json<Value>) -> HttpResult<Json<Value>> {
    Ok(Json(json!({
        "message": "Hello from echo_post!",
        "request": body
    })))
}
