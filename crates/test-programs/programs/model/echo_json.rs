//! The echo default shapes a `format::json` answer as an object.

#![cfg(target_arch = "wasm32")]

use omnia_sdk::WasiProvider;
use omnia_sdk::model::{Format, Model as _, Request};
use test_programs::user;

omnia_sdk::export!(command: scenario);

async fn scenario() {
    let reply = WasiProvider
        .complete(Request::builder().messages(vec![user("hi")]).format(Format::Json).build())
        .await
        .expect("echo answers json");
    assert_eq!(reply.answer, r#"{"echo":"hi"}"#);
}
