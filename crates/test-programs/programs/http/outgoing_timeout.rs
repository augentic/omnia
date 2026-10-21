//! Two outbound GETs under the guest's own `request-options` timeouts, against
//! an origin that stalls: a head that never comes and a body that stops short
//! of its length. Each is `connection-read-timeout`, not a connect timeout or
//! a bare connection failure, and the short body's bytes arrive before the
//! code does.

#![cfg(target_arch = "wasm32")]

use omnia_sdk::Config;
use omnia_sdk::http::{Method, Request};
use wasip3::http::client;
use wasip3::http::types::{ErrorCode, RequestOptions};
use wasip3::http_compat::{IncomingMessage as _, RequestOptionsExtension, http_into_wasi_request};
use wasip3::wit_future;

omnia_sdk::command!(scenario);

struct WasiConfig;

impl Config for WasiConfig {}

// nanoseconds, as `wasi:clocks` spells a duration
const TIMEOUT: u64 = 200_000_000;

async fn scenario() {
    let origin = WasiConfig.get("ORIGIN").await.expect("ORIGIN seeded");

    // the head never comes; the connect is bounded too, as the first byte is
    // awaited on top of it
    let options = RequestOptions::new();
    options.set_connect_timeout(Some(TIMEOUT)).expect("connect timeout");
    options.set_first_byte_timeout(Some(TIMEOUT)).expect("first-byte timeout");
    let error = omnia_wasi_http::handle(request(&origin, "/head", options))
        .await
        .expect_err("a head that never comes must time out");
    let code = error.downcast_ref::<ErrorCode>().expect("a wasi:http error code");
    assert!(matches!(code, ErrorCode::ConnectionReadTimeout), "got {code:?}");

    // the body stops short; its error rides the trailers future, which
    // `omnia_wasi_http::handle` does not await, so the raw bindings read it
    let options = RequestOptions::new();
    options.set_between_bytes_timeout(Some(TIMEOUT)).expect("between-bytes timeout");
    let wasi_request =
        http_into_wasi_request(request(&origin, "/body", options)).expect("wasi request");
    let response = client::send(wasi_request).await.expect("the head arrives");
    assert_eq!(response.get_status_code(), 200);
    let (_, body_result) = wit_future::new(|| Ok(()));
    let (stream, trailers) = response.consume_body(body_result);
    assert_eq!(stream.collect().await, b"12345678");
    let code = trailers.await.expect_err("a body that stops short must time out");
    assert!(matches!(code, ErrorCode::ConnectionReadTimeout), "got {code:?}");
}

fn request(origin: &str, path: &str, options: RequestOptions) -> Request<String> {
    let mut request = Request::builder()
        .method(Method::GET)
        .uri(format!("{origin}{path}"))
        .body(String::new())
        .expect("request");
    request.extensions_mut().insert(RequestOptionsExtension(options));
    request
}
