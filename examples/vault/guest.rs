//! # Vault Wasm Guest
//!
//! This module demonstrates the WASI Vault interface for secure secret storage.
//! It shows how to:
//! - Open a vault "locker" (namespace for secrets)
//! - Store secrets securely
//! - Retrieve secrets by key
//!
//! ## Backend Agnostic
//!
//! This guest code works with any WASI Vault backend:
//! - In-memory (this example's host)
//! - Azure Key Vault (vault-azure example)
//! - HashiCorp Vault
//! - AWS Secrets Manager
//!
//! ## Security
//!
//! Secrets stored via WASI Vault are:
//! - Encrypted at rest (backend-dependent)
//! - Access-controlled by the host
//! - Never exposed in logs or traces

#![cfg(target_arch = "wasm32")]

use anyhow::Context;
use axum::routing::post;
use axum::{Json, Router};
use bytes::Bytes;
use qwasr_sdk::HttpResult;
use qwasr_wasi_vault::vault;
use serde_json::Value;
use tracing::Level;
use wasip3::exports::http::handler::Guest;
use wasip3::http::types::{ErrorCode, Request, Response};

struct Http;
wasip3::http::service::export!(Http);

impl Guest for Http {
    /// Routes incoming requests to the vault handler.
    #[qwasr_wasi_otel::instrument(name = "http_guest_handle", level = Level::DEBUG)]
    async fn handle(request: Request) -> Result<Response, ErrorCode> {
        let router = Router::new().route("/", post(handler));
        qwasr_wasi_http::serve(router, request).await
    }
}

/// Stores and retrieves a secret from the vault.
#[qwasr_wasi_otel::instrument]
async fn handler(body: Bytes) -> HttpResult<Json<Value>> {
    let locker =
        vault::open("qwasr-locker".to_string()).await.context("failed to open vault locker")?;

    locker.set("secret-id".to_string(), body.to_vec()).await.context("issue setting secret")?;

    let secret = locker.get("secret-id".to_string()).await.context("issue retriving secret")?;
    assert_eq!(secret.unwrap(), body);

    let response = serde_json::from_slice::<Value>(&body).context("deserializing data")?;
    tracing::debug!("sending response: {response:?}");
    Ok(Json(response))
}
