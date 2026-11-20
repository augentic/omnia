//! `Request::complete_checked` with a hand-written check: the callback needs
//! no `Question`. The check accepts only `hi`, correcting anything else.

#![cfg(target_arch = "wasm32")]

use std::future::ready;

use omnia_sdk::model::{Candidate, Request, ToolCall};
use test_programs::user;

omnia_sdk::command::export!(scenario);

async fn scenario() {
    let request = Request::builder().messages(vec![user("hi")]).build();

    let mut candidates = Vec::new();
    let reply = request
        .complete_checked(
            |call: ToolCall| {
                ready(Err::<String, _>(format!("no tool `{}` is declared", call.name)))
            },
            |candidate: Candidate| {
                candidates.push(candidate.answer.clone());
                ready(if candidate.answer == "hi" {
                    Ok(())
                } else {
                    Err(format!("say `hi`, not `{}`", candidate.answer))
                })
            },
        )
        .await
        .expect("the completion runs")
        .expect("a candidate is accepted");

    assert_eq!(reply.answer, "hi");
    assert_eq!(candidates.last().map(String::as_str), Some("hi"));
}
