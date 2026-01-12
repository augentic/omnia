//! # Config Wasm Guest
//!
//! This module demonstrates the WASI Config interface for retrieving
//! configuration variables.

#![cfg(target_arch = "wasm32")]

use axum::routing::get;
use axum::{Json, Router};
use serde_json::{Value, json};
use wasip3::exports::http::handler::Guest;
use wasip3::http::types::{ErrorCode, Request, Response};
use yetti_sdk::HttpResult;
use yetti_wasi_config::store as config;

struct HttpGuest;
wasip3::http::proxy::export!(HttpGuest);

impl Guest for HttpGuest {
    async fn handle(request: Request) -> Result<Response, ErrorCode> {
        let router = Router::new().route("/", get(config_get));
        yetti_wasi_http::serve(router, request).await
    }
}

/// Config request handler.
#[yetti_wasi_otel::instrument]
async fn config_get() -> HttpResult<Json<Value>> {
    let config = config::get_all().expect("should get all");

    Ok(Json(json!({
        "config": config
    })))
}
