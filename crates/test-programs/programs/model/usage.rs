//! A backend-reported usage record is projected onto the guest reply.

#![cfg(target_arch = "wasm32")]

use omnia_sdk::model::Request;
use test_programs::user;

omnia_sdk::command::export!(scenario);

async fn scenario() {
    let reply = Request::builder()
        .messages(vec![user("hi")])
        .build()
        .complete()
        .await
        .expect("the completion runs")
        .expect("canned answer carries usage");
    assert_eq!(reply.answer, "hi");

    let usage = reply.usage.expect("the backend reports usage");
    assert_eq!(
        (
            usage.input_tokens,
            usage.output_tokens,
            usage.reasoning_tokens,
            usage.cache_read_tokens,
            usage.cache_write_tokens,
        ),
        (3, 5, Some(1), Some(2), None)
    );
}
