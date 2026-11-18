//! # Vault Wasm Guest
//!
//! This module demonstrates the `omnia:vault` interface for secure secret
//! storage. It shows how to:
//! - Open a locker (namespace for secrets) the deployment declares
//! - Store secrets securely
//! - Retrieve secrets by name
//!
//! ## Backend Agnostic
//!
//! This guest code works with any `omnia:vault` backend:
//! - In-memory (this example's host)
//! - Azure Key Vault (`omnia-azure-vault` in `omnia-backends`)
//!
//! ## Security
//!
//! Secrets stored via `omnia:vault` are:
//! - Encrypted at rest (backend-dependent)
//! - Access-controlled by the host
//! - Never exposed in logs or traces

#![cfg(target_arch = "wasm32")]

use anyhow::Context;
use axum::routing::post;
use axum::{Json, Router};
use bytes::Bytes;
use omnia_wasi_vault::omnia::store;
use serde_json::Value;
use tracing::Level;
use wasip3::exports::http::handler::Guest;
use wasip3::http::types::{ErrorCode, Request, Response};

struct Http;
wasip3::http::service::export!(Http);

impl Guest for Http {
    #[omnia_wasi_otel::instrument(name = "http_guest_handle", level = Level::DEBUG)]
    async fn handle(request: Request) -> Result<Response, ErrorCode> {
        let router = Router::new().route("/", post(handler));
        omnia_wasi_http::serve(router, request).await
    }
}

#[omnia_wasi_otel::instrument]
async fn handler(body: Bytes) -> omnia_sdk::Result<Json<Value>> {
    let locker = store::open("default".to_string()).await.context("failed to open vault locker")?;

    locker.set("greeting".to_string(), body.to_vec()).await.context("issue setting secret")?;

    let secret = locker.get("greeting".to_string()).await.context("issue retrieving secret")?;
    assert_eq!(secret.unwrap(), body);

    let response = serde_json::from_slice::<Value>(&body).context("deserializing data")?;
    tracing::debug!("sending response: {response:?}");
    Ok(Json(response))
}
