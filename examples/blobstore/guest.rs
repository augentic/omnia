//! # Blobstore Wasm Guest
//!
//! This module demonstrates the `omnia:blobstore` interface for storing and
//! retrieving binary data (blobs) through the `omnia_sdk::blobstore` bindings.
//! It shows how to:
//! - Create containers (namespaces for blobs)
//! - Put an object in a container
//! - Get the object back
//!
//! ## Backend Flexibility
//!
//! This guest code works with any `omnia:blobstore` backend:
//! - In-memory (this example's host)
//! - MongoDB (blobstore-mongodb example)
//! - NATS Object Store (blobstore-nats example)

#![cfg(target_arch = "wasm32")]

use anyhow::{Context, anyhow};
use axum::routing::post;
use axum::{Json, Router};
use bytes::Bytes;
use omnia_sdk::blobstore;
use serde_json::Value;
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
    // store the request body as an object
    let container = blobstore::create("container").await.context("creating container")?;
    container.write("request", &body).await.context("writing data")?;

    // read the object back
    let data = container
        .read("request")
        .await
        .context("reading data")?
        .ok_or_else(|| anyhow!("object not found"))?;

    // verify the round-trip was successful (an error, not a guest trap)
    if data != body {
        return Err(anyhow!(
            "round-trip mismatch: read {} bytes, wrote {}",
            data.len(),
            body.len()
        )
        .into());
    }

    let response = serde_json::from_slice::<Value>(&data).context("deserializing data")?;
    Ok(Json(response))
}
