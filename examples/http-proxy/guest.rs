//! # HTTP Proxy Wasm Guest
//!
//! This module demonstrates an HTTP proxy pattern with caching using WASI HTTP.
//! It shows how to:
//! - Make outbound HTTP requests from a WebAssembly guest
//! - Implement HTTP caching with ETags and Cache-Control headers
//! - Use client certificates for mTLS authentication
//!
//! ## Caching Strategy
//!
//! The proxy uses standard HTTP caching headers:
//! - `Cache-Control`: Controls caching duration (`max-age`) and behavior (`no-cache`)
//! - `If-None-Match`: Provides an ETag for cache lookup
//!
//! ## Endpoints
//!
//! - `GET /echo`: Simple echo handler
//! - `GET /cache`: Fetch with caching (returns cached response if available)
//! - `POST /origin`: Fetch from origin, cache response
//! - `POST /client-cert`: Fetch with client certificate authentication

#![cfg(target_arch = "wasm32")]

use std::convert::Infallible;

use anyhow::Context;
use axum::body::Body;
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use base64ct::{Base64, Encoding};
use bytes::Bytes;
use http::Method;
use http::header::{CACHE_CONTROL, IF_NONE_MATCH};
use http_body_util::Empty;
use qwasr_sdk::HttpResult;
use qwasr_wasi_http::CacheOptions;
use serde_json::{Value, json};
use tracing::Level;
use wasip3::exports::http::handler::Guest;
use wasip3::http::types::{ErrorCode, Request, Response};

struct HttpGuest;
wasip3::http::service::export!(HttpGuest);

impl Guest for HttpGuest {
    /// Routes incoming requests to appropriate handlers.
    #[qwasr_wasi_otel::instrument(name = "http_guest_handle", level = Level::DEBUG)]
    async fn handle(request: Request) -> Result<Response, ErrorCode> {
        let router = Router::new()
            .route("/echo", get(echo))
            .route("/cache", get(cache))
            .route("/origin", post(origin))
            .route("/client-cert", post(client_cert));
        qwasr_wasi_http::serve(router, request).await
    }
}

/// Simple echo handler that returns the request body with a greeting.
#[qwasr_wasi_otel::instrument]
async fn echo(Json(body): Json<Value>) -> HttpResult<Json<Value>> {
    Ok(Json(json!({
        "message": "Hello, World!",
        "request": body
    })))
}

/// Fetches data with HTTP caching enabled.
#[qwasr_wasi_otel::instrument]
async fn cache() -> Result<impl IntoResponse, Infallible> {
    let request = http::Request::builder()
        .method(Method::GET)
        .uri("https://jsonplaceholder.cypress.io/posts/1")
        .header(CACHE_CONTROL, "max-age=300")
        .header(IF_NONE_MATCH, "qf55low9rjsrup46vsiz9r73")
        .extension(CacheOptions {
            bucket_name: "example-bucket".to_string(),
        })
        .body(Empty::<Bytes>::new())
        .expect("failed to build request");

    let response = qwasr_wasi_http::handle(request).await.unwrap();
    let (parts, body) = response.into_parts();
    let http_response = http::Response::from_parts(parts, Body::from(body));

    Ok(http_response)
}

/// Fetches from origin and caches the response.
#[qwasr_wasi_otel::instrument]
async fn origin() -> HttpResult<Json<Value>> {
    let request = http::Request::builder()
        .method(Method::GET)
        .uri("https://jsonplaceholder.cypress.io/posts")
        .header(CACHE_CONTROL, "no-cache, max-age=300")
        .body(Empty::<Bytes>::new())
        .expect("failed to build request");

    let response = qwasr_wasi_http::handle(request).await?;
    let body = response.into_body();
    let body = serde_json::from_slice::<Value>(&body).context("issue parsing response body")?;

    Ok(Json(body))
}

/// Demonstrates mTLS client certificate authentication.
#[qwasr_wasi_otel::instrument]
async fn client_cert() -> HttpResult<Json<Value>> {
    let auth_cert = "
        -----BEGIN CERTIFICATE-----
        ...Your Certificate Here...
        -----END CERTIFICATE----- 
        -----BEGIN PRIVATE KEY-----
        ...Your Private Key Here...
        -----END PRIVATE KEY-----";
    let encoded_cert = Base64::encode_string(auth_cert.as_bytes());

    let request = http::Request::builder()
        .method(Method::GET)
        .uri("https://jsonplaceholder.cypress.io/posts/1")
        .header("Client-Cert", &encoded_cert)
        .extension(CacheOptions {
            bucket_name: "example-bucket".to_string(),
        })
        .body(Empty::<Bytes>::new())
        .expect("Failed to build request");

    let response = qwasr_wasi_http::handle(request).await?;
    let body = response.into_body();
    let body_str = Base64::encode_string(&body);

    Ok(Json(serde_json::json!({
        "cached_response": body_str
    })))
}
