//! # Identity Wasm Guest
//!
//! This module demonstrates `omnia:identity` for obtaining authentication
//! credentials. It shows how to:
//! - Open the identity `default` the host declares
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
use omnia_sdk::identity;
use serde_json::{Value, json};
use wasip3::exports::http::handler::Guest;
use wasip3::http::types::{ErrorCode, Request, Response};

const SCOPE: &str = "https://management.azure.com/.default";

struct Http;
wasip3::http::service::export!(Http);

impl Guest for Http {
    async fn handle(request: Request) -> Result<Response, ErrorCode> {
        let router = Router::new().route("/", get(handler));
        omnia_sdk::http::serve(router, request).await
    }
}

#[omnia_sdk::otel::instrument]
async fn handler() -> omnia_sdk::Result<Json<Value>> {
    let credentials = identity::default().await.context("opening identity")?;

    let access_token = credentials.token(&[SCOPE]).await.context("getting access token")?;

    // Never print the token itself; report only enough to confirm success.
    println!(
        "access token acquired ({} chars, expires in {}s)",
        access_token.token.len(),
        access_token.expires_in().as_secs()
    );

    Ok(Json(json!({
        "message": "Hello, World!"
    })))
}
