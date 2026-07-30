//! # Routing example — guest A
//!
//! A minimal HTTP guest that identifies itself as `a`. The deployment manifest
//! (`omnia.toml`) routes the `/a` path prefix to this guest.

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

/// Respond to any path with this guest's identity and the `HTTP_ADDR` it
/// sees (the seam suite asserts the runtime's injected value reaches the
/// guest environment).
async fn respond() -> String {
    let addr = std::env::var("HTTP_ADDR").unwrap_or_else(|_| "unset".into());
    format!("http-routing example: guest a (HTTP_ADDR={addr})\n")
}
