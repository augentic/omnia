//! One outbound GET to a TLS origin whose certificate no root trusts: the
//! host refuses the handshake, and the guest sees `tls-certificate-error`
//! rather than a bare connection failure.

#![cfg(target_arch = "wasm32")]

use http::{Method, Request};
use omnia_sdk::config;
use wasip3::http::types::ErrorCode;

omnia_sdk::api::command::export!(scenario);

async fn scenario() {
    let origin: String = config::require("ORIGIN").await.expect("ORIGIN seeded");

    let request = Request::builder()
        .method(Method::GET)
        .uri(format!("{origin}/secure"))
        .body(String::new())
        .expect("request");
    let error =
        omnia_sdk::http::send(request).await.expect_err("untrusted certificate must be refused");
    let code = error.downcast_ref::<ErrorCode>().expect("a wasi:http error code");
    assert!(matches!(code, ErrorCode::TlsCertificateError), "got {code:?}");
}
