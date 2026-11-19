//! # Config Wasm Guest
//!
//! This module demonstrates the `omnia:config` interface for listing
//! configuration variables.

#![cfg(target_arch = "wasm32")]

use axum::routing::get;
use axum::{Json, Router};
use omnia_wasi_config::omnia::entries;
use serde_json::{Value, json};
use wasip3::exports::http::handler::Guest;
use wasip3::http::types::{ErrorCode, Request, Response};

struct HttpGuest;
wasip3::http::service::export!(HttpGuest);

impl Guest for HttpGuest {
    async fn handle(request: Request) -> Result<Response, ErrorCode> {
        let router = Router::new().route("/", get(config_get));
        omnia_wasi_http::serve(router, request).await
    }
}

#[omnia_wasi_otel::instrument]
async fn config_get() -> omnia_sdk::Result<Json<Value>> {
    let page = entries::list(String::new(), None, None).await?;

    Ok(Json(json!({
        "config": page.entries
    })))
}
