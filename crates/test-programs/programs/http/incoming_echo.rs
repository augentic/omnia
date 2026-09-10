//! A `wasi:http/incoming-handler` guest behind an `omnia_guest::api::http`
//! router: the one route echoes a `message` taken from the JSON body (POST)
//! or the query string (GET), alongside the request id the transport
//! metadata carried in. The handler first asserts the host's request
//! normalisation reached it: scheme and authority filled from `Host`.

#![cfg(target_arch = "wasm32")]

use axum::Router;
use omnia_guest::Error;
use omnia_guest::api::http::{get, post};
use omnia_guest::api::{Client, Context};
use serde::{Deserialize, Serialize};
use wasip3::exports::http::handler::Guest;
use wasip3::http::types::{ErrorCode, Request, Response, Scheme};

struct HttpGuest;
wasip3::http::service::export!(HttpGuest);

impl Guest for HttpGuest {
    async fn handle(request: Request) -> Result<Response, ErrorCode> {
        assert!(matches!(request.get_scheme(), Some(Scheme::Http)));
        assert_eq!(request.get_authority().as_deref(), Some("echo.test"));

        let router = Router::new()
            .route("/echo", get(echo).merge(post(echo)))
            .with_state(Client::new("test-programs", Provider));
        omnia_wasi_http::serve(router, request).await
    }
}

struct Provider;

#[derive(Deserialize)]
struct Input {
    message: String,
}

#[derive(Serialize)]
struct Echo {
    message: String,
    request_id: String,
}

async fn echo(input: Input, context: Context<Provider>) -> Result<Echo, Error> {
    let request_id = context.metadata.request_id.expect("http metadata always carries an id");
    Ok(Echo {
        message: input.message,
        request_id,
    })
}
