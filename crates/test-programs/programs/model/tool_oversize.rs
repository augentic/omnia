//! A tool result over the host's byte cap fails the completion.

#![cfg(target_arch = "wasm32")]

use omnia_guest::model::{Error, Model as _, Request, WasiModel};
use test_programs::{lookup, user};

omnia_guest::command!(scenario);

async fn scenario() {
    let request = Request::builder().messages(vec![user("hi")]).tools(vec![lookup()]).build();

    let error = WasiModel
        .complete_with(request, |_call| async { Ok::<_, String>("hello".to_owned()) })
        .await
        .expect_err("oversize result fails the completion");

    assert!(
        matches!(error, Error::ToolFailed(ref detail) if detail.contains("exceeds the 4-byte cap")),
        "unexpected: {error:?}"
    );
}
