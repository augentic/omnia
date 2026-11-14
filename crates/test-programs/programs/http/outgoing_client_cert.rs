//! Outbound GETs carrying a `Client-Cert` bundle, one per leaf shape. A leaf
//! usable for TLS client authentication is accepted and the request reaches
//! the origin (the identity engages only on TLS, so a plain-HTTP origin still
//! answers):
//! - a client-authentication certificate
//! - a certificate with no extended key usage
//! - a client-authentication leaf followed by its CA
//!
//! Every other leaf is refused by the host before any connection is
//! attempted, and the guest sees an `internal-error` naming the reason:
//! - a server-authentication certificate
//! - a CA certificate
//! - a key usage that excludes signing
//! - an expired certificate
//! - a certificate not yet valid

#![cfg(target_arch = "wasm32")]

use http::{Method, Request, StatusCode};
use omnia_sdk::{ConfigExt as _, WasiProvider};
use wasip3::http::types::ErrorCode;

omnia_sdk::api::command::export!(scenario);

const ACCEPTED: [(&str, &str); 3] = [
    ("CLIENT_CERT_OK", "/mtls"),
    ("CLIENT_CERT_NO_EKU", "/no-eku"),
    ("CLIENT_CERT_CHAIN", "/chain"),
];

const REFUSED: [(&str, &str, &str); 5] = [
    (
        "CLIENT_CERT_SERVER",
        "/server-auth",
        "extended key usage does not permit client authentication",
    ),
    ("CLIENT_CERT_CA", "/ca", "is a CA certificate"),
    ("CLIENT_CERT_KEY_ENCIPHERMENT", "/key-encipherment", "key usage does not permit signing"),
    ("CLIENT_CERT_EXPIRED", "/expired", "has expired"),
    ("CLIENT_CERT_NOT_YET_VALID", "/not-yet-valid", "is not yet valid"),
];

async fn scenario() {
    let origin: String = WasiProvider.require("ORIGIN").await.expect("ORIGIN seeded");

    for (key, path) in ACCEPTED {
        let request = request(&origin, path, key).await;
        let response = omnia_wasi_http::handle(request)
            .await
            .unwrap_or_else(|error| panic!("{key} refused: {error:?}"));
        assert_eq!(response.status(), StatusCode::OK, "{key}");
        assert_eq!(response.body().as_ref(), b"hello from origin", "{key}");
    }

    for (key, path, reason) in REFUSED {
        let request = request(&origin, path, key).await;
        let error = omnia_wasi_http::handle(request).await.expect_err(key);
        let code = error.downcast_ref::<ErrorCode>().expect("a wasi:http error code");
        assert!(
            matches!(code, ErrorCode::InternalError(Some(message)) if message.contains(reason)),
            "{key}: got {code:?}"
        );
    }
}

async fn request(origin: &str, path: &str, key: &str) -> Request<String> {
    let bundle: String = WasiProvider.require(key).await.unwrap_or_else(|_| panic!("{key} seeded"));
    Request::builder()
        .method(Method::GET)
        .uri(format!("{origin}{path}"))
        .header("Client-Cert", bundle)
        .body(String::new())
        .expect("request")
}
