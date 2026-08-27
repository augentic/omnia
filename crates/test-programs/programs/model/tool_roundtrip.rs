//! A declared tool call round-trips through the `complete_with` handler.

#![cfg(target_arch = "wasm32")]

use omnia_guest::model::{Model as _, Request, ToolCall, WasiModel};
use test_programs::{lookup, user};

test_programs::run!(scenario);

async fn scenario() {
    let request = Request::builder().messages(vec![user("hi")]).tools(vec![lookup()]).build();

    let mut calls: Vec<ToolCall> = Vec::new();
    let reply = WasiModel
        .complete_with(request, |call| {
            calls.push(call);
            async { Ok::<_, String>("42".to_owned()) }
        })
        .await
        .expect("tool loop answers");

    assert_eq!(reply.answer, "42");
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].name, "lookup");
    assert_eq!(calls[0].arguments, "{}");
    assert!(!calls[0].id.is_empty());
}
