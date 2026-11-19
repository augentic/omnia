//! Without a workspace grant the host-injected tools refuse to run.

#![cfg(target_arch = "wasm32")]

use omnia_sdk::model::{Error, Request};
use test_programs::user;

omnia_sdk::api::command::export!(scenario);

async fn scenario() {
    let error = Request::builder()
        .messages(vec![user("hi")])
        .build()
        .complete()
        .await
        .expect_err("workspace tools require a grant");
    assert!(
        matches!(error, Error::Backend(ref detail) if detail.contains("requires grants.workspace")),
        "unexpected: {error:?}"
    );
}
