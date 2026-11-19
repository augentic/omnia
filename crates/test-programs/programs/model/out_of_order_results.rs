//! Raw session bindings: two parallel calls answered in reverse order still
//! correlate by id, not arrival order.

#![cfg(target_arch = "wasm32")]

use omnia_sdk::model::{completion, types};
use omnia_sdk::wit_stream;
use test_programs::{raw_lookup, raw_model, raw_request};

omnia_sdk::api::command::export!(scenario);

async fn scenario() {
    let request = raw_request(vec![raw_lookup()], types::Grants { workspace: None });
    let model = raw_model().await;
    let (mut results, results_rx) = wit_stream::new();
    let session = model.complete(request, results_rx).await.expect("session opens");
    let completion::Session { mut calls, reply } = session;

    let Some(types::Call::Tool(first)) = calls.next().await else {
        panic!("first call is a tool call");
    };
    let Some(types::Call::Tool(second)) = calls.next().await else {
        panic!("second call is a tool call");
    };
    assert_ne!(first.id, second.id);

    // Answer in reverse order; each output echoes its call's arguments,
    // so the backend's in-order reply proves correlation by id.
    let _ = results
        .write_one(types::CallResult::Tool(types::ToolResult {
            id: second.id.clone(),
            output: Ok(second.arguments.clone()),
        }))
        .await;
    let _ = results
        .write_one(types::CallResult::Tool(types::ToolResult {
            id: first.id.clone(),
            output: Ok(first.arguments.clone()),
        }))
        .await;

    let reply = reply.await.expect("both calls resolve").expect("the model answers");
    assert_eq!(reply.answer, "1|2");
}
