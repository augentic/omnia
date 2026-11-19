//! Raw session bindings: a result with an unknown id is dropped, not fatal.

#![cfg(target_arch = "wasm32")]

use omnia_wasi_model::omnia::{completion, types};
use omnia_wasi_model::wit_stream;
use test_programs::{raw_lookup, raw_model, raw_request};

omnia_sdk::api::command::export!(scenario);

async fn scenario() {
    let request = raw_request(
        vec![raw_lookup()],
        types::Grants {
            workspace: None,
            worker: None,
        },
    );
    let model = raw_model().await;
    let (mut results, results_rx) = wit_stream::new();
    let session = model.complete(request, results_rx).await.expect("session opens");
    let completion::Session { mut calls, reply } = session;

    let Some(types::Call::Tool(call)) = calls.next().await else {
        panic!("model issues a tool call");
    };
    let _ = results
        .write_one(types::CallResult::Tool(types::ToolResult {
            id: "nope".to_owned(),
            output: Ok("ignored".to_owned()),
        }))
        .await;
    let _ = results
        .write_one(types::CallResult::Tool(types::ToolResult {
            id: call.id.clone(),
            output: Ok("42".to_owned()),
        }))
        .await;

    let reply = reply.await.expect("unknown id is dropped").expect("the model answers");
    assert_eq!(reply.answer, "42");
}
