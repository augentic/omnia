//! A question hands the backend's other tool calls to the guest's tools, and
//! refuses each one when the guest passes none.

#![cfg(target_arch = "wasm32")]

use omnia_sdk::model::{Question, ToolCall, Tools};
use serde::Deserialize;
use test_programs::{Verdict, lookup};

omnia_sdk::command::export!(scenario);

#[derive(Deserialize)]
struct Lookup {
    id: u32,
}

async fn scenario() {
    let question = Question::<Verdict>::new("verdict").tools(vec![lookup()]);

    let tools: Tools = Box::new(|call: ToolCall| {
        Box::pin(async move {
            let Lookup { id } = call.arguments()?;
            Ok(format!("looked up {id}"))
        })
    });
    let verdict = question.ask("judge this", Some(tools), Verdict::passing).await;
    assert_eq!(verdict.expect("the handled call leaves a verdict").verdict, "pass");

    let verdict = question.ask("judge this", None, Verdict::passing).await;
    assert_eq!(verdict.expect("the refused call leaves a verdict").verdict, "pass");
}
