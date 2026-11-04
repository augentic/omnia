//! A `wasi:http/incoming-handler` guest behind `omnia_sdk::http!`,
//! `omnia_sdk::api::http` routing, and `WasiProvider`: the one route echoes a
//! `message` taken from the JSON body (POST) or the query string (GET),
//! alongside the request id the transport metadata carried in. Middleware
//! first asserts the host's request normalisation reached it: scheme and
//! authority filled from `Host`.

#![cfg(target_arch = "wasm32")]

use axum::Router;
use axum::body::Body;
use axum::http::Request;
use axum::http::uri::Authority;
use axum::middleware::map_request;
use omnia_sdk::api::http::{get, post};
use omnia_sdk::api::{Client, Context};
use omnia_sdk::{Error, WasiProvider};
use serde::{Deserialize, Serialize};

omnia_sdk::http!(router(WasiProvider));

fn router<P: Send + Sync + 'static>(provider: P) -> Router {
    Router::new()
        .route("/echo", get(echo).merge(post(echo)))
        .layer(map_request(check_uri))
        .with_state(Client::new("test-programs", provider))
}

async fn check_uri(request: Request<Body>) -> Request<Body> {
    assert_eq!(request.uri().scheme_str(), Some("http"));
    assert_eq!(request.uri().authority().map(Authority::as_str), Some("echo.test"));
    request
}

#[derive(Deserialize)]
struct Input {
    message: String,
}

#[derive(Serialize)]
struct Echo {
    message: String,
    request_id: String,
}

async fn echo<P>(input: Input, context: Context<P>) -> Result<Echo, Error> {
    let request_id = context.metadata.request_id.expect("http metadata always carries an id");
    Ok(Echo {
        message: input.message,
        request_id,
    })
}
