//! A backend call to an undeclared tool fails the completion loud, as
//! `other`.

#![cfg(target_arch = "wasm32")]

use omnia_sdk::model::Request;
use test_programs::{fails_with, user};

omnia_sdk::command::export!(scenario);

async fn scenario() {
    // No tools declared; the host rejects the backend's `lookup` call before
    // the guest ever sees it.
    let error = Request::builder()
        .messages(vec![user("hi")])
        .build()
        .complete()
        .await
        .expect_err("undeclared tool fails the completion");
    fails_with(&error, "other", "does not declare");
}
