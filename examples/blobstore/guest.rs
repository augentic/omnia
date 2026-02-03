//! # Blobstore Wasm Guest
//!
//! This module demonstrates the WASI Blobstore interface for storing and
//! retrieving binary data (blobs). It shows how to:
//! - Create containers (namespaces for blobs)
//! - Write data using streaming `OutgoingValue`
//! - Read data using `IncomingValue`
//!
//! ## Backend Flexibility
//!
//! This guest code works with any WASI Blobstore backend:
//! - In-memory (this example's host)
//! - MongoDB (blobstore-mongodb example)
//! - NATS Object Store (blobstore-nats example)

#![cfg(target_arch = "wasm32")]

use anyhow::anyhow;
use axum::routing::post;
use axum::{Json, Router};
use bytes::Bytes;
use qwasr_sdk::HttpResult;
use qwasr_wasi_blobstore::blobstore;
use qwasr_wasi_blobstore::types::{IncomingValue, OutgoingValue};
use serde_json::Value;
use tracing::Level;
use wasip3::exports::http::handler::Guest;
use wasip3::http::types::{ErrorCode, Request, Response};

struct Http;
wasip3::http::service::export!(Http);

impl Guest for Http {
    /// Routes incoming HTTP requests to the blob storage handler.
    #[qwasr_wasi_otel::instrument(name = "http_guest_handle", level = Level::DEBUG)]
    async fn handle(request: Request) -> Result<Response, ErrorCode> {
        let router = Router::new().route("/", post(handler));
        qwasr_wasi_http::serve(router, request).await
    }
}

/// Stores and retrieves data from the blobstore.
#[qwasr_wasi_otel::instrument]
async fn handler(body: Bytes) -> HttpResult<Json<Value>> {
    // create an outgoing value to hold the data we want to store
    let outgoing = OutgoingValue::new_outgoing_value();
    let stream = outgoing
        .outgoing_value_write_body()
        .await
        .map_err(|()| anyhow!("failed to create stream"))?;
    stream.blocking_write_and_flush(&body).map_err(|e| anyhow!("writing body: {e}"))?;

    // write the blob to the container
    let container = blobstore::create_container("container".to_string())
        .await
        .map_err(|e| anyhow!("failed to create container: {e}"))?;
    container
        .write_data("request".to_string(), &outgoing)
        .await
        .map_err(|e| anyhow!("failed to write data: {e}"))?;

    OutgoingValue::finish(outgoing).map_err(|e| anyhow!("issue finishing: {e}"))?;

    // read the blob back from the container
    let incoming = container
        .get_data("request".to_string(), 0, 0)
        .await
        .map_err(|e| anyhow!("failed to read data: {e}"))?;
    let data = IncomingValue::incoming_value_consume_sync(incoming)
        .map_err(|e| anyhow!("failed to create incoming value: {e}"))?;

    // verify the round-trip was successful
    assert_eq!(data, body);

    let response =
        serde_json::from_slice::<Value>(&data).map_err(|e| anyhow!("deserializing data: {e}"))?;
    Ok(Json(response))
}
