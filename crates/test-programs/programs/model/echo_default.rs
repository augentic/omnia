//! The echo default, over the models the deployment declares by name.
//!
//! - A request that names none opens `default`, a declared name answers, and
//!   an undeclared name is `not-found`.
//! - A `format::json` answer is shaped as an object.
//! - A checked completion offers the prompt as its one candidate.
//! - What the echo cannot honour, `format::schema` and a generation control,
//!   is each `unsupported`.

#![cfg(target_arch = "wasm32")]

use std::future::ready;

use omnia_sdk::model::{Candidate, Format, Generation, Request, ToolCall};
use test_programs::{VERDICT_SCHEMA, fails_with, user, verdict_format};

omnia_sdk::command::export!(scenario);

async fn scenario() {
    // a declared name answers, and no name opens `default`
    for name in [Some("fast"), None] {
        let request = Request::builder().maybe_model(name).messages(vec![user("hi")]).build();
        let reply = request
            .complete()
            .await
            .expect("the completion runs")
            .expect("a declared model answers");
        assert_eq!(reply.answer, "hi", "{name:?}");
    }
    let request = Request::builder().model("slow").messages(vec![user("hi")]).build();
    let error = request.complete().await.expect_err("an undeclared model fails");
    fails_with(&error, "not-found", "`slow`");

    // a json answer is an object
    let reply = Request::builder()
        .messages(vec![user("hi")])
        .format(Format::Json)
        .build()
        .complete()
        .await
        .expect("the completion runs")
        .expect("echo answers json");
    assert_eq!(reply.answer, r#"{"echo":"hi"}"#);

    // a checked completion's one candidate is the prompt
    let mut candidates = Vec::new();
    let reply = Request::builder()
        .messages(vec![user("hi")])
        .build()
        .complete_checked(
            |call: ToolCall| {
                ready(Err::<String, _>(format!("no tool `{}` is declared", call.name)))
            },
            |candidate: Candidate| {
                candidates.push(candidate.answer.clone());
                ready(Ok::<(), String>(()))
            },
        )
        .await
        .expect("the completion runs")
        .expect("the candidate is accepted");
    assert_eq!(reply.answer, "hi");
    assert_eq!(candidates, ["hi"]);

    // what the echo cannot honour is refused
    let schema = Request::builder()
        .messages(vec![user("hi")])
        .format(verdict_format(VERDICT_SCHEMA))
        .build();
    let generation = Request::builder()
        .messages(vec![user("hi")])
        .generation(Generation {
            temperature: Some(0.25),
            ..Generation::default()
        })
        .build();
    for (request, needle) in
        [(schema, "cannot satisfy format::schema"), (generation, "ignores generation controls")]
    {
        let error = request.complete().await.expect_err("the echo refuses");
        fails_with(&error, "unsupported", needle);
    }
}
