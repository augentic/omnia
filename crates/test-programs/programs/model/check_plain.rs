//! `Request::check` with a hand-written handler: the callback needs no
//! `Question`. The handler accepts only `hi`, correcting anything else.

#![cfg(target_arch = "wasm32")]

use std::future::ready;

use omnia_sdk::model::{CHECK_TOOL, Request};
use test_programs::user;

omnia_sdk::command::export!(scenario);

async fn scenario() {
    let request = Request::builder().messages(vec![user("hi")]).check(true).build();

    let mut candidates = Vec::new();
    let reply = request
        .complete_with(|call| {
            assert_eq!(call.name, CHECK_TOOL);
            candidates.push(call.arguments.clone());
            ready(if call.arguments == "hi" {
                Ok(String::new())
            } else {
                Err(format!("say `hi`, not `{}`", call.arguments))
            })
        })
        .await
        .expect("a candidate is accepted");

    assert_eq!(reply.answer, "hi");
    assert_eq!(candidates.last().map(String::as_str), Some("hi"));
}
