//! Every `omnia:model` outcome reaches the guest by its case: a host error
//! is an SDK error under the case's name, and `exhausted` is the outcome the
//! completion answers with. The suite's faulting backend fails a completion
//! by the model it names. The host refuses an empty model name before it
//! reaches the backend, and fails a session answered with the wrong kind of
//! result.

#![cfg(target_arch = "wasm32")]

use omnia_sdk::model::{self, Budget, Request, completion, types};
use omnia_sdk::wit_stream;
use test_programs::{lookup, raw_model, raw_request, user};

omnia_sdk::command::export!(scenario);

async fn scenario() {
    // the backend's chosen cases, through the SDK
    for name in ["not-found", "denied", "invalid", "unsupported", "unavailable", "other"] {
        let request = Request::builder().model(name).messages(vec![user("hi")]).build();
        let error = request.complete().await.expect_err("the backend fails the completion");
        assert_eq!(error.code(), name, "unexpected: {error:?}");
    }
    let request = Request::builder().model("exhausted").messages(vec![user("hi")]).build();
    let exhausted = request
        .complete()
        .await
        .expect("the completion runs")
        .expect_err("the backend spends a budget");
    assert_eq!(exhausted.budget, Budget::Tokens, "unexpected: {exhausted:?}");

    // the host's own rules
    let error = model::open("").await.expect_err("the host refuses an empty name");
    assert_eq!(error.code(), "invalid", "unexpected: {error:?}");

    // a verdict for a tool call
    let model = raw_model().await;
    let (mut results, results_rx) = wit_stream::new();
    let session = model
        .complete(raw_request(vec![lookup()], types::Grants { workspace: None }), results_rx)
        .await
        .expect("session opens");
    let completion::Session { mut calls, reply } = session;
    let Some(types::Call::Tool(call)) = calls.next().await else {
        panic!("model issues a tool call");
    };
    let verdict = types::Verdict {
        id: call.id,
        outcome: Ok(()),
    };
    let _ = results.write_one(types::CallResult::Check(verdict)).await;
    let error = reply.await.expect_err("the host fails the session");
    assert!(matches!(error, types::Error::Invalid(_)), "unexpected: {error:?}");
}
