//! One outbound GET through `omnia_wasi_http::handle` against the host's
//! loopback origin: the status, marker header, and body the origin answered
//! arrive intact, and no `ETag` is minted when caching is not requested.

#![cfg(target_arch = "wasm32")]

use omnia_sdk::http::header::ETAG;
use omnia_sdk::http::{Method, Request, StatusCode};
use omnia_sdk::{ConfigExt as _, WasiProvider};

omnia_sdk::api::command::export!(scenario);

async fn scenario() {
    let origin: String = WasiProvider.require("ORIGIN").await.expect("ORIGIN seeded");

    let request = Request::builder()
        .method(Method::GET)
        .uri(format!("{origin}/resource"))
        .header("x-probe", "1")
        .body(String::new())
        .expect("request");
    let response = omnia_wasi_http::handle(request).await.expect("handle");

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers().get("x-mock").and_then(|v| v.to_str().ok()), Some("origin"));
    assert!(response.headers().get(ETAG).is_none(), "no cache, no etag");
    assert_eq!(response.body().as_ref(), b"hello from origin");
}
