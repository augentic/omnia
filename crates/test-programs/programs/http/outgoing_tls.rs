//! One outbound GET to a TLS origin whose certificate no root trusts: the
//! host refuses the handshake, and the guest sees `tls-certificate-error`
//! rather than a bare connection failure.

#![cfg(target_arch = "wasm32")]

use omnia_sdk::http::{Method, Request};
use omnia_sdk::{Config, WasiProvider};
use wasip3::http::types::ErrorCode;

omnia_sdk::command!(scenario);

async fn scenario() {
    let origin = WasiProvider.get("ORIGIN").await.expect("ORIGIN seeded");

    let request = Request::builder()
        .method(Method::GET)
        .uri(format!("{origin}/secure"))
        .body(String::new())
        .expect("request");
    let error =
        omnia_wasi_http::handle(request).await.expect_err("untrusted certificate must be refused");
    let code = error.downcast_ref::<ErrorCode>().expect("a wasi:http error code");
    assert!(matches!(code, ErrorCode::TlsCertificateError), "got {code:?}");
}
