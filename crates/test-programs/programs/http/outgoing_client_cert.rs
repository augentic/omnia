//! Two outbound GETs carrying a `Client-Cert` bundle: a client-authentication
//! certificate is accepted and the request reaches the origin (the identity
//! engages only on TLS, so a plain-HTTP origin still answers); a
//! server-authentication certificate is refused by the host before any
//! connection is attempted.

#![cfg(target_arch = "wasm32")]

use omnia_sdk::Config;
use omnia_sdk::http::{Method, Request, StatusCode};

omnia_sdk::command!(scenario);

struct WasiConfig;

impl Config for WasiConfig {}

async fn scenario() {
    let origin = WasiConfig.get("ORIGIN").await.expect("ORIGIN seeded");
    let client_cert = WasiConfig.get("CLIENT_CERT_OK").await.expect("CLIENT_CERT_OK seeded");
    let server_cert =
        WasiConfig.get("CLIENT_CERT_SERVER").await.expect("CLIENT_CERT_SERVER seeded");

    let request = Request::builder()
        .method(Method::GET)
        .uri(format!("{origin}/mtls"))
        .header("Client-Cert", &client_cert)
        .body(String::new())
        .expect("request");
    let response = omnia_wasi_http::handle(request).await.expect("client-auth bundle accepted");
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.body().as_ref(), b"hello from origin");

    let request = Request::builder()
        .method(Method::GET)
        .uri(format!("{origin}/mtls"))
        .header("Client-Cert", &server_cert)
        .body(String::new())
        .expect("request");
    assert!(omnia_wasi_http::handle(request).await.is_err(), "server-auth bundle must be refused");
}
