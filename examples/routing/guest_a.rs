//! # Routing example — guest A
//!
//! A minimal HTTP guest that identifies itself as `a`. The deployment manifest
//! (`omni.toml`) routes the `/a` path prefix to this guest.

#![cfg(target_arch = "wasm32")]

use axum::Router;
use wasip3::exports::http::handler::Guest;
use wasip3::http::types::{ErrorCode, Request, Response};

struct GuestA;
wasip3::http::service::export!(GuestA);

impl Guest for GuestA {
    async fn handle(request: Request) -> Result<Response, ErrorCode> {
        let router = Router::new().fallback(respond);
        omnia_wasi_http::serve(router, request).await
    }
}

/// Respond to any path with this guest's identity.
async fn respond() -> &'static str {
    "routing example: guest a\n"
}
