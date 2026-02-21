//! # HTTP Proxy Wasm Guest

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
use omnia_sdk::HttpResult;
use omnia_wasi_http::CacheOptions;
use serde_json::Value;
use tracing::Level;
use wasip3::exports::http::handler::Guest;
use wasip3::http::types::{ErrorCode, Request, Response};

struct HttpGuest;
wasip3::http::service::export!(HttpGuest);

impl Guest for HttpGuest {
    /// Routes incoming requests to appropriate handlers.
    #[omnia_wasi_otel::instrument(name = "http_guest_handle", level = Level::DEBUG)]
    async fn handle(request: Request) -> Result<Response, ErrorCode> {
        let router = Router::new()
            .route("/cache", get(cache))
            .route("/origin-sm", get(origin_sm))
            .route("/origin-xl", post(origin_xl))
            .route("/client-cert", post(client_cert));
        omnia_wasi_http::serve(router, request).await
    }
}

/// Fetches data with HTTP caching enabled.
#[omnia_wasi_otel::instrument]
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

    let response = omnia_wasi_http::handle(request).await.unwrap();
    let (parts, body) = response.into_parts();
    let http_response = http::Response::from_parts(parts, Body::from(body));

    Ok(http_response)
}

#[omnia_wasi_otel::instrument]
async fn origin_sm() -> Result<impl IntoResponse, Infallible> {
    tracing::info!("fetching from origin-sm");

    let request = http::Request::builder()
        .method(Method::GET)
        .uri("https://jsonplaceholder.cypress.io/posts/1")
        .body(Empty::<Bytes>::new())
        .expect("failed to build request");

    let response = omnia_wasi_http::handle(request).await.unwrap();
    let (parts, body) = response.into_parts();
    let http_response = http::Response::from_parts(parts, Body::from(body));

    tracing::info!("fetched from origin-sm");
    Ok(http_response)
}

/// Fetches from origin and caches the response.
#[omnia_wasi_otel::instrument]
async fn origin_xl() -> HttpResult<Json<Value>> {
    tracing::info!("fetching from origin-xl");

    let request = http::Request::builder()
        .method(Method::GET)
        .uri("https://jsonplaceholder.cypress.io/posts")
        .header(CACHE_CONTROL, "no-cache, max-age=300")
        .body(Empty::<Bytes>::new())
        .expect("failed to build request");

    let response = omnia_wasi_http::handle(request).await?;
    let body = response.into_body();
    let body = serde_json::from_slice::<Value>(&body).context("issue parsing response body")?;

    tracing::info!("fetched from origin-xl");
    Ok(Json(body))
}

/// Demonstrates mTLS client certificate authentication.
#[omnia_wasi_otel::instrument]
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

    let response = omnia_wasi_http::handle(request).await?;
    let body = response.into_body();
    let body_str = Base64::encode_string(&body);

    Ok(Json(serde_json::json!({
        "cached_response": body_str
    })))
}
