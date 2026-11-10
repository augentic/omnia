//! A `wasi:http/incoming-handler` guest behind `http::export!`, plain axum
//! handlers, and `WasiProvider`: the one route echoes a `message` taken from
//! the JSON body (POST) or the query string (GET), alongside the request id
//! the `Context` extractor carried in. Middleware first asserts the host's
//! request normalisation reached it: scheme and authority filled from `Host`.

#![cfg(target_arch = "wasm32")]

use axum::Router;
use axum::body::Body;
use axum::extract::{Json, Query};
use axum::http::Request;
use axum::http::uri::Authority;
use axum::middleware::map_request;
use axum::routing::get;
use omnia_sdk::Error;
use omnia_sdk::api::{Client, Context, http};
use serde::{Deserialize, Serialize};

#[derive(Deserialize)]
struct Input {
    message: String,
}

#[derive(Serialize)]
struct Echo {
    message: String,
    request_id: String,
}

http::export!(router);

fn router<P: Send + Sync + 'static>(provider: P) -> Router {
    Router::new()
        .route("/echo", get(echo_get).post(echo_post))
        .layer(map_request(check_uri))
        .with_state(Client::new(provider))
}

async fn check_uri(request: Request<Body>) -> Request<Body> {
    assert_eq!(request.uri().scheme_str(), Some("http"));
    assert_eq!(request.uri().authority().map(Authority::as_str), Some("echo.test"));
    request
}

async fn echo_get<P: Send + Sync + 'static>(
    context: Context<P>, Query(input): Query<Input>,
) -> Result<Json<Echo>, Error> {
    echo(input, context)
}

async fn echo_post<P: Send + Sync + 'static>(
    context: Context<P>, Json(input): Json<Input>,
) -> Result<Json<Echo>, Error> {
    echo(input, context)
}

fn echo<P>(input: Input, context: Context<P>) -> Result<Json<Echo>, Error> {
    let request_id = context.metadata.request_id.expect("http metadata always carries an id");
    Ok(Json(Echo {
        message: input.message,
        request_id,
    }))
}
