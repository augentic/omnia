//! # Routing example — guest B
//!
//! A minimal HTTP guest that identifies itself as `b`. The deployment manifest
//! (`omni.toml`) routes the `/b` path prefix to this guest.

#![cfg(target_arch = "wasm32")]

use axum::Router;
use wasip3::exports::http::handler::Guest;
use wasip3::http::types::{ErrorCode, Request, Response};

struct GuestB;
wasip3::http::service::export!(GuestB);

impl Guest for GuestB {
    async fn handle(request: Request) -> Result<Response, ErrorCode> {
        let router = Router::new().fallback(respond);
        omnia_wasi_http::serve(router, request).await
    }
}

/// Respond to any path with this guest's identity.
async fn respond() -> &'static str {
    "routing example: guest b\n"
}
