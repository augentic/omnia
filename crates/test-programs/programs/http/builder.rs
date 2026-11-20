//! Outbound requests built with `omnia_sdk::http`'s builder. A JSON `POST`
//! under a bearer token reaches the origin whole, and the origin's echo
//! decodes back to what was sent. A reply that is not JSON fails as a bad
//! gateway, and a timeout bounds a request whose head never comes.

#![cfg(target_arch = "wasm32")]

use std::time::Duration;

use ::http::StatusCode;
use omnia_sdk::{config, http};
use serde_json::{Value, json};

omnia_sdk::command::export!(scenario);

async fn scenario() {
    let origin: String = config::require("ORIGIN").await.expect("ORIGIN seeded");
    let stalling: String = config::require("STALLING").await.expect("STALLING seeded");

    let order = json!({ "sku": "a-1", "quantity": 2 });
    let echoed: Value = http::post(format!("{origin}/orders"))
        .bearer("t0ken")
        .json(&order)
        .fetch_json()
        .await
        .expect("the echo decodes");
    assert_eq!(echoed, order);

    let err = http::get(format!("{origin}/resource"))
        .fetch_json::<Value>()
        .await
        .expect_err("a plain-text body is not JSON");
    assert_eq!(err.status(), StatusCode::BAD_GATEWAY, "{err}");

    let err = http::get(format!("{stalling}/head"))
        .timeout(Duration::from_millis(200))
        .fetch()
        .await
        .expect_err("a head that never comes must time out");
    assert_eq!(err.code(), "unavailable", "{err}");
    assert!(err.description().contains("ConnectionReadTimeout"), "{err}");
}
