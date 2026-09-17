//! # HTTP Proxy Wasm Guest

#![cfg(target_arch = "wasm32")]

use anyhow::Context;
use axum::Router;
use axum::body::Body;
use axum::response::IntoResponse;
use axum::routing::get;
use bytes::Bytes;
use http::Method;
use http_body_util::Empty;
use omnia_sdk::HttpResult;
use tracing::Level;
use wasip3::exports::http::handler::Guest;
use wasip3::http::types::{ErrorCode, Request, Response};

struct HttpGuest;
wasip3::http::service::export!(HttpGuest);

impl Guest for HttpGuest {
    #[omnia_wasi_otel::instrument(name = "http_guest_handle", level = Level::DEBUG)]
    async fn handle(request: Request) -> Result<Response, ErrorCode> {
        let router = Router::new().route("/origin-sm", get(origin_sm));
        omnia_wasi_http::serve(router, request).await
    }
}

#[omnia_wasi_otel::instrument]
async fn origin_sm() -> HttpResult<impl IntoResponse> {
    tracing::info!("fetching from origin-sm");

    let request = http::Request::builder()
        .method(Method::GET)
        .uri("https://jsonplaceholder.cypress.io/posts/1")
        .body(Empty::<Bytes>::new())
        .context("building request")?;

    let response = omnia_wasi_http::handle(request).await?;
    let (parts, body) = response.into_parts();

    tracing::info!("fetched from origin-sm");
    Ok(http::Response::from_parts(parts, Body::from(body)))
}
