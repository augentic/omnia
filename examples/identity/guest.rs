//! # Identity Wasm Guest
//!
//! This module demonstrates the WASI Identity interface for obtaining
//! authentication credentials. It shows how to:
//! - Retrieve an identity provider from the host
//! - Request access tokens with specific scopes
//!
//! ## Use Cases
//!
//! - Authenticating to cloud APIs (Azure, AWS, GCP)
//! - Service-to-service authentication
//! - Obtaining tokens for downstream API calls

#![cfg(target_arch = "wasm32")]

use anyhow::Context;
use axum::routing::get;
use axum::{Json, Router};
use omnia_sdk::HttpResult;
use omnia_wasi_identity::credentials::get_identity;
use serde_json::{Value, json};
use tracing::Level;
use wasip3::exports::http::handler::Guest;
use wasip3::http::types::{ErrorCode, Request, Response};
use wit_bindgen::block_on;

const SCOPE: &str = "https://management.azure.com/.default";

struct Http;
wasip3::http::service::export!(Http);

impl Guest for Http {
    /// Routes incoming requests to the identity handler.
    #[omnia_wasi_otel::instrument(name = "http_guest_handle", level = Level::INFO)]
    async fn handle(request: Request) -> Result<Response, ErrorCode> {
        let router = Router::new().route("/", get(handler));
        omnia_wasi_http::serve(router, request).await
    }
}

/// Obtains an access token from the identity provider.
#[omnia_wasi_otel::instrument]
async fn handler() -> HttpResult<Json<Value>> {
    let identity = block_on(get_identity("identity".to_string())).context("getting identity")?;

    let scopes = vec![SCOPE.to_string()];
    let access_token = block_on(async move { identity.get_token(scopes).await })
        .context("getting access token")?;

    println!("access token: {}", access_token.token);

    Ok(Json(json!({
        "message": "Hello, World!"
    })))
}
