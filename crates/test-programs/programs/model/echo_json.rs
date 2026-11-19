//! The echo default shapes a `format::json` answer as an object.

#![cfg(target_arch = "wasm32")]

use omnia_sdk::model::{Format, Request};
use test_programs::user;

omnia_sdk::command::export!(scenario);

async fn scenario() {
    let reply = Request::builder()
        .messages(vec![user("hi")])
        .format(Format::Json)
        .build()
        .complete()
        .await
        .expect("echo answers json");
    assert_eq!(reply.answer, r#"{"echo":"hi"}"#);
}
