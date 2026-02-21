//! # KeyValue Store Wasm Guest
//!
//! This module demonstrates the WASI KeyValue interface. It shows how to:
//! - Open a named bucket (key-value namespace)
//! - Store and retrieve data by key
//! - Combine HTTP handling with storage operations
//!
//! ## Backend Flexibility
//!
//! This guest code works with any WASI KeyValue backend:
//! - In-memory (this example's host)
//! - Redis (keyvalue-redis example)
//! - NATS JetStream (keyvalue-nats example)

#![cfg(target_arch = "wasm32")]

use anyhow::Context;
use axum::routing::post;
use axum::{Json, Router};
use bytes::Bytes;
use omnia_sdk::HttpResult;
use omnia_wasi_keyvalue::store;
use serde_json::{Value, json};
use tracing::Level;
use wasip3::exports::http::handler::Guest;
use wasip3::http::types::{ErrorCode, Request, Response};

struct Http;
wasip3::http::service::export!(Http);

impl Guest for Http {
    /// Routes incoming HTTP requests to the key-value handler.
    #[omnia_wasi_otel::instrument(name = "http_guest_handle", level = Level::INFO)]
    async fn handle(request: Request) -> Result<Response, ErrorCode> {
        let router = Router::new().route("/", post(handler));
        omnia_wasi_http::serve(router, request).await
    }
}

/// Stores and retrieves data from the key-value store.
#[omnia_wasi_otel::instrument]
async fn handler(body: Bytes) -> HttpResult<Json<Value>> {
    let bucket = store::open("omnia_bucket".to_string()).await.context("opening bucket")?;

    bucket.set("my_key".to_string(), body.to_vec()).await.context("storing data")?;

    let res = bucket.get("my_key".to_string()).await.context("reading data")?;
    tracing::debug!("found val: {res:?}");

    Ok(Json(json!({
        "message": "Hello, World!"
    })))
}
