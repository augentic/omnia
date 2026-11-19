//! The echo default answers a text completion with the last user turn.

#![cfg(target_arch = "wasm32")]

use omnia_sdk::model::Request;
use test_programs::user;

omnia_sdk::api::command::export!(scenario);

async fn scenario() {
    let reply = Request::builder()
        .system("be terse")
        .messages(vec![user("hi"), user("second")])
        .build()
        .complete()
        .await
        .expect("echo answers");
    assert_eq!(reply.answer, "second");
    assert!(reply.usage.is_none());
}
