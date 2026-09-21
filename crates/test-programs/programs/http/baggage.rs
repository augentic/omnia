//! A `wasi:http/incoming-handler` guest rooting a server chain: it sets one
//! baggage entry, dispatches `ping-async` to the echoer, and answers with the
//! entry the echoer was dispatched with — a trigger-served guest's baggage
//! reaching its callee. The link call runs before the router so the answer
//! is the body whatever the path.

#![cfg(target_arch = "wasm32")]

wit_bindgen::generate!({
    world: "caller",
    path: "wit",
    generate_all,
});

use axum::Router;
use axum::routing::get;
use omnia_test::link::ops;
use wasip3::exports::http::handler::Guest;
use wasip3::http::types::{ErrorCode, Request, Response};

struct HttpGuest;
wasip3::http::service::export!(HttpGuest);

impl Guest for HttpGuest {
    async fn handle(request: Request) -> Result<Response, ErrorCode> {
        omnia_wasi_otel::set_baggage([("tenant", "acme")]);
        let answer = ops::ping_async("echoer".to_owned(), "tenant".to_owned()).await;

        let router = Router::new().route("/baggage", get(move || async move { answer }));
        omnia_wasi_http::serve(router, request).await
    }
}
