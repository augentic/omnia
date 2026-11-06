//! Outbound GETs carrying `Client-Cert` bundles: client-authentication
//! certificates (standalone, no EKU, or with a signing CA chain) are accepted
//! and reach the origin; CA leaves, non-signing or server-only certificates,
//! expired or not-yet-valid certificates, and malformed PEM bundles are
//! refused by the host before any connection is attempted.

#![cfg(target_arch = "wasm32")]

use omnia_sdk::http::{Method, Request, StatusCode};
use omnia_sdk::{Config, WasiProvider};

omnia_sdk::api::command::export!(scenario);

async fn scenario() {
    let origin = WasiProvider.get("ORIGIN").await.expect("ORIGIN seeded");

    for key in ["CLIENT_CERT_OK", "CLIENT_CERT_NO_EKU", "CLIENT_CERT_CHAIN"] {
        let cert = WasiProvider.get(key).await.expect("accepted bundle seeded");
        let request = Request::builder()
            .method(Method::GET)
            .uri(format!("{origin}/mtls"))
            .header("Client-Cert", &cert)
            .body(String::new())
            .expect("request");
        let response = omnia_wasi_http::handle(request).await.expect("accepted bundle");
        assert_eq!(response.status(), StatusCode::OK, "{key}");
        assert_eq!(response.body().as_ref(), b"hello from origin", "{key}");
    }

    for key in [
        "CLIENT_CERT_SERVER",
        "CLIENT_CERT_CA",
        "CLIENT_CERT_NO_SIGN",
        "CLIENT_CERT_EXPIRED",
        "CLIENT_CERT_FUTURE",
        "CLIENT_CERT_KEY_ONLY",
        "CLIENT_CERT_GARBAGE",
    ] {
        let cert = WasiProvider.get(key).await.expect("rejected bundle seeded");
        let request = Request::builder()
            .method(Method::GET)
            .uri(format!("{origin}/mtls"))
            .header("Client-Cert", &cert)
            .body(String::new())
            .expect("request");
        assert!(omnia_wasi_http::handle(request).await.is_err(), "{key} must be refused");
    }
}
