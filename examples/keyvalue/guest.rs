//! # KeyValue Store Wasm Guest
//!
//! This module demonstrates the `omnia:keyvalue` interface. It shows how to:
//! - Open a named bucket (key-value namespace)
//! - Store and retrieve data by key
//! - Update an entry only while it holds the revision it was read at
//! - Combine HTTP handling with storage operations
//!
//! ## Backend Flexibility
//!
//! This guest code works with any `omnia:keyvalue` backend:
//! - In-memory (this example's host)
//! - Redis (keyvalue-redis example)
//! - NATS JetStream (keyvalue-nats example)

#![cfg(target_arch = "wasm32")]

use anyhow::{Context, anyhow};
use axum::routing::post;
use axum::{Json, Router};
use bytes::Bytes;
use omnia_sdk::keyvalue;
use serde_json::{Value, json};
use wasip3::exports::http::handler::Guest;
use wasip3::http::types::{ErrorCode, Request, Response};

struct Http;
wasip3::http::service::export!(Http);

impl Guest for Http {
    async fn handle(request: Request) -> Result<Response, ErrorCode> {
        let router = Router::new().route("/", post(handler));
        omnia_sdk::http::serve(router, request).await
    }
}

#[omnia_sdk::otel::instrument]
async fn handler(body: Bytes) -> omnia_sdk::Result<Json<Value>> {
    let bucket = keyvalue::open("omnia_bucket").await.context("opening bucket")?;

    bucket.write("my_key", &body, None).await.context("storing data")?;

    let res = bucket.get("my_key".to_string()).await.context("reading data")?;
    tracing::debug!("found entry: {res:?}");

    // Optimistic update: write only while the entry holds the revision it was
    // read at. A write that slips in between is a conflict carrying the entry
    // it left, so the one retry starts from that.
    let entry = res.context("`my_key` is missing")?;
    let update = |revision| bucket.compare_and_swap("my_key", Some(revision), b"updated", None);
    if let Err(conflict) = update(entry.revision).await.context("updating data")? {
        let current = conflict.current.context("`my_key` was deleted")?;
        if update(current.revision).await.context("retrying the update")?.is_err() {
            return Err(anyhow!("the retried update lost its race too").into());
        }
    }

    Ok(Json(json!({
        "message": "Hello, World!"
    })))
}
