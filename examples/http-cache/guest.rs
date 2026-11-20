//! # HTTP Cache Wasm Guest
//!
//! Serves `GET /cached` by fetching `{UPSTREAM_URL}/resource` through
//! `HttpCache`. The first call reaches the origin, and later calls within
//! `max-age` are answered from the `default` keyvalue bucket. Either way the
//! response carries `ETag: "v1"`.

#![cfg(target_arch = "wasm32")]

use anyhow::Context as _;
use axum::body::Body;
use axum::response::IntoResponse;
use axum::routing::get;
use bytes::Bytes;
use http::Method;
use http::header::{CACHE_CONTROL, IF_NONE_MATCH};
use omnia_sdk::http::cache::HttpCache;
use omnia_sdk::{Handlers, Router, config, keyvalue};

omnia_sdk::http::export!(router);

fn router() -> Router {
    Handlers::new().route("/cached", get(cached)).into_router()
}

#[omnia_sdk::otel::instrument]
async fn cached() -> omnia_sdk::Result<impl IntoResponse> {
    let upstream: String = config::require("UPSTREAM_URL").await?;
    tracing::info!("fetching {upstream}/resource through the cache");

    let request = http::Request::builder()
        .method(Method::GET)
        .uri(format!("{upstream}/resource"))
        .header(CACHE_CONTROL, "max-age=300")
        .header(IF_NONE_MATCH, "\"v1\"")
        .body(Bytes::new())
        .context("building request")?;

    let bucket = keyvalue::open(keyvalue::DEFAULT).await?;
    let response = HttpCache::new(bucket).fetch(request).await?;
    let (parts, body) = response.into_parts();

    Ok(http::Response::from_parts(parts, Body::from(body)))
}
