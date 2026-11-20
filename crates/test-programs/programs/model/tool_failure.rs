//! A handler `Err` is repair input the model sees, not a session failure.

#![cfg(target_arch = "wasm32")]

use omnia_sdk::model::Request;
use test_programs::{lookup, user};

omnia_sdk::command::export!(scenario);

async fn scenario() {
    let request = Request::builder().messages(vec![user("hi")]).tools(vec![lookup()]).build();

    let reply = request
        .complete_with(|_call| async { Err::<String, _>("no data".to_owned()) })
        .await
        .expect("the completion runs")
        .expect("the model turns the failure into an answer");
    assert_eq!(reply.answer, "tool failed: no data");
}
