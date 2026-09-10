//! Two identical GETs under `Cache-Control: max-age=60` with a strong
//! `If-None-Match`: the first fills the guest-side cache from the origin,
//! the second is served from it with the same status, headers, and body,
//! and both carry the request's etag as `ETag`.

#![cfg(target_arch = "wasm32")]

use omnia_guest::Config;
use omnia_guest::http::header::{CACHE_CONTROL, ETAG, IF_NONE_MATCH};
use omnia_guest::http::{Method, Request, StatusCode};

omnia_guest::command!(scenario);

struct WasiConfig;

impl Config for WasiConfig {}

async fn scenario() {
    let origin = WasiConfig.get("ORIGIN").await.expect("ORIGIN seeded");

    let mut responses = Vec::new();
    for _ in 0..2 {
        let request = Request::builder()
            .method(Method::GET)
            .uri(format!("{origin}/cached"))
            .header(CACHE_CONTROL, "max-age=60")
            .header(IF_NONE_MATCH, "\"v1\"")
            .body(String::new())
            .expect("request");
        let response = omnia_wasi_http::handle(request).await.expect("handle");

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers().get(ETAG).and_then(|v| v.to_str().ok()), Some("\"v1\""));
        assert_eq!(response.headers().get("x-mock").and_then(|v| v.to_str().ok()), Some("origin"));
        assert_eq!(response.body().as_ref(), b"hello from origin");
        responses.push(response);
    }

    // The cached copy is the origin's response, headers included.
    assert_eq!(responses[0].headers(), responses[1].headers());
    assert_eq!(responses[0].body(), responses[1].body());
}
