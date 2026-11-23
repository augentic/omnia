//! A declared tool's calls over a chain of completions, through the SDK and
//! the raw session bindings.
//!
//! - A call round-trips through the `complete_with` handler.
//! - A handler `Err` is repair input the model sees, not a session failure.
//! - Dropping the results writer before answering is `invalid`, not a
//!   timeout.
//! - Dropping the calls stream with a call outstanding is `invalid`, not a
//!   timeout.
//! - A result with an unknown id is dropped, not fatal.

#![cfg(target_arch = "wasm32")]

use omnia_sdk::model::{Request, ToolCall, completion, types};
use omnia_sdk::wit_stream;
use test_programs::{lookup, raw_model, raw_request, user};

omnia_sdk::command::export!(scenario);

async fn scenario() {
    roundtrip().await;
    failure().await;
    results_closed().await;
    calls_dropped().await;
    stale_result().await;
}

async fn roundtrip() {
    let request = Request::builder().messages(vec![user("hi")]).tools(vec![lookup()]).build();
    let mut calls: Vec<ToolCall> = Vec::new();
    let reply = request
        .complete_with(|call| {
            calls.push(call);
            async { Ok::<_, String>("42".to_owned()) }
        })
        .await
        .expect("the completion runs")
        .expect("tool loop answers");
    assert_eq!(reply.answer, "42");
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].name, "lookup");
    assert_eq!(calls[0].arguments, "{}");
    assert!(!calls[0].id.is_empty());
}

async fn failure() {
    let request = Request::builder().messages(vec![user("hi")]).tools(vec![lookup()]).build();
    let reply = request
        .complete_with(|_call| async { Err::<String, _>("no data".to_owned()) })
        .await
        .expect("the completion runs")
        .expect("the model turns the failure into an answer");
    assert_eq!(reply.answer, "tool failed: no data");
}

async fn results_closed() {
    let request = raw_request(vec![lookup()], types::Grants { workspace: None });
    let model = raw_model().await;
    let (results, results_rx) = wit_stream::new();
    let session = model.complete(request, results_rx).await.expect("session opens");
    let completion::Session { mut calls, reply } = session;

    let Some(types::Call::Tool(call)) = calls.next().await else {
        panic!("model issues a tool call");
    };
    assert_eq!(call.name, "lookup");

    drop(results);

    let error = reply.await.expect_err("closed results stream fails the completion");
    assert!(
        matches!(error, types::Error::Invalid(ref detail) if detail.contains("closed its results stream")),
        "unexpected: {error:?}"
    );
}

async fn calls_dropped() {
    let request = raw_request(vec![lookup()], types::Grants { workspace: None });
    let model = raw_model().await;
    let (results, results_rx) = wit_stream::new();
    let session = model.complete(request, results_rx).await.expect("session opens");
    let completion::Session { mut calls, reply } = session;

    let Some(types::Call::Tool(call)) = calls.next().await else {
        panic!("model issues a tool call");
    };
    assert_eq!(call.name, "lookup");

    // Holding the results writer open leaves the dropped calls stream as
    // the only cause.
    drop(calls);
    let outcome = reply.await;
    drop(results);

    let error = outcome.expect_err("dropped calls stream fails the completion");
    assert!(
        matches!(error, types::Error::Invalid(ref detail) if detail.contains("dropped its calls stream")),
        "unexpected: {error:?}"
    );
}

async fn stale_result() {
    let request = raw_request(vec![lookup()], types::Grants { workspace: None });
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
