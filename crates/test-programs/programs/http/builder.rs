//! Outbound requests through `omnia_sdk::http`. A request sent as built
//! reaches the host's loopback origin, and the status, marker header, and
//! body the origin answered arrive intact, with no `ETag` minted when caching
//! is not requested. A JSON `POST` from the builder, under a bearer token,
//! reaches the origin whole, and the origin's echo decodes back to what was
//! sent. A reply that is not JSON fails as a bad gateway, and a timeout
//! bounds a request whose head never comes.

#![cfg(target_arch = "wasm32")]

use std::time::Duration;

use ::http::header::ETAG;
use ::http::{Method, Request, StatusCode};
use omnia_sdk::{config, http};
use serde_json::{Value, json};

omnia_sdk::command::export!(scenario);

async fn scenario() {
    let origin: String = config::require("ORIGIN").await.expect("ORIGIN seeded");
    let stalling: String = config::require("STALLING").await.expect("STALLING seeded");

    let request = Request::builder()
        .method(Method::GET)
        .uri(format!("{origin}/resource"))
        .header("x-probe", "1")
        .body(String::new())
        .expect("request");
    let response = http::send(request).await.expect("handle");
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers().get("x-mock").and_then(|v| v.to_str().ok()), Some("origin"));
    assert!(response.headers().get(ETAG).is_none(), "no cache, no etag");
    assert_eq!(response.body().as_ref(), b"hello from origin");

    let order = json!({ "sku": "a-1", "quantity": 2 });
    let echoed: Value = http::post(format!("{origin}/orders"))
        .bearer("t0ken")
        .json(&order)
        .fetch_json()
        .await
        .expect("the echo decodes");
    assert_eq!(echoed, order);

    let err = http::get(format!("{origin}/resource"))
        .fetch_json::<Value>()
        .await
        .expect_err("a plain-text body is not JSON");
    assert_eq!(err.status(), StatusCode::BAD_GATEWAY, "{err}");

    let err = http::get(format!("{stalling}/head"))
        .timeout(Duration::from_millis(200))
        .fetch()
        .await
        .expect_err("a head that never comes must time out");
    assert_eq!(err.code(), "unavailable", "{err}");
    assert!(err.description().contains("ConnectionReadTimeout"), "{err}");
}
