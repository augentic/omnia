//! Outbound GETs under a `hosts` grant that reaches `localhost`, against the
//! host's loopback origin by name and by address. `localhost` answers, and a
//! redirect within it is followed. The loopback address is
//! `http-request-denied` however it is spelled, whether the guest names it
//! or a redirect from `localhost` leads there.

#![cfg(target_arch = "wasm32")]

use http::{Method, Request, StatusCode};
use omnia_sdk::config;
use wasip3::http::types::ErrorCode;

omnia_sdk::command::export!(scenario);

async fn scenario() {
    let port: String = config::require("PORT").await.expect("PORT seeded");
    let by_name = format!("http://localhost:{port}");
    let by_address = format!("http://127.0.0.1:{port}");

    let response = omnia_sdk::http::send(get(&format!("{by_name}/resource")))
        .await
        .expect("`localhost` is granted");
    assert_eq!(response.status(), StatusCode::OK);

    let response = omnia_sdk::http::send(get(&format!("{by_name}/redirect?to={by_name}/resource")))
        .await
        .expect("a redirect within the grant is followed");
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.body().as_ref(), b"hello from origin");

    let refused = [
        format!("{by_address}/resource"),
        format!("http://2130706433:{port}/resource"),
        format!("http://127.0.0.1.:{port}/resource"),
        format!("{by_name}/redirect?to={by_address}/resource"),
    ];
    for url in refused {
        let error =
            omnia_sdk::http::send(get(&url)).await.expect_err("an address is outside the grant");
        let code = error.downcast_ref::<ErrorCode>().expect("a wasi:http error code");
        assert!(matches!(code, ErrorCode::HttpRequestDenied), "`{url}` got {code:?}");
    }
}

fn get(url: &str) -> Request<String> {
    Request::builder().method(Method::GET).uri(url).body(String::new()).expect("request")
}
