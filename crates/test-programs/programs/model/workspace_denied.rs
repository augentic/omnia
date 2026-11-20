//! Without a workspace grant the host-injected tools refuse to run.

#![cfg(target_arch = "wasm32")]

use omnia_sdk::model::Request;
use test_programs::{fails_with, user};

omnia_sdk::command::export!(scenario);

async fn scenario() {
    let error = Request::builder()
        .messages(vec![user("hi")])
        .build()
        .complete()
        .await
        .expect_err("workspace tools require a grant");
    fails_with(&error, "other", "requires grants.workspace");
}
