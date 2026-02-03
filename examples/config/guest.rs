//! # Config Wasm Guest
//!
//! This module demonstrates the WASI Config interface for retrieving
//! configuration variables.

#![cfg(target_arch = "wasm32")]

use axum::routing::get;
use axum::{Json, Router};
use qwasr_sdk::HttpResult;
use qwasr_wasi_config::store as config;
use serde_json::{Value, json};
use wasip3::exports::http::handler::Guest;
use wasip3::http::types::{ErrorCode, Request, Response};

struct HttpGuest;
wasip3::http::service::export!(HttpGuest);

impl Guest for HttpGuest {
    async fn handle(request: Request) -> Result<Response, ErrorCode> {
        let router = Router::new().route("/", get(config_get));
        qwasr_wasi_http::serve(router, request).await
    }
}

/// Config request handler.
#[qwasr_wasi_otel::instrument]
async fn config_get() -> HttpResult<Json<Value>> {
    let config = config::get_all().expect("should get all");

    Ok(Json(json!({
        "config": config
    })))
}
