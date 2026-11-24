//! Two outbound GETs under a `hosts` grant of `prices.test` alone.
//! `prices.test` answers, and `other.test` is `http-request-denied`.

#![cfg(target_arch = "wasm32")]

use http::{Method, Request, StatusCode};
use wasip3::http::types::ErrorCode;

omnia_sdk::command::export!(scenario);

async fn scenario() {
    let response = omnia_sdk::http::send(get("https://prices.test/items/7"))
        .await
        .expect("`prices.test` is granted");
    assert_eq!(response.status(), StatusCode::OK);

    let error = omnia_sdk::http::send(get("https://other.test/items/7"))
        .await
        .expect_err("`other.test` is outside the grant");
    let code = error.downcast_ref::<ErrorCode>().expect("a wasi:http error code");
    assert!(matches!(code, ErrorCode::HttpRequestDenied), "got {code:?}");
}

fn get(url: &str) -> Request<String> {
    Request::builder().method(Method::GET).uri(url).body(String::new()).expect("request")
}
