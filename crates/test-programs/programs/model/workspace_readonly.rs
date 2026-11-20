//! A read-only mount lets the backend read, but write is refused.

#![cfg(target_arch = "wasm32")]

use omnia_sdk::model::Request;
use test_programs::{fails_with, user};
use wasip3::filesystem::preopens;

omnia_sdk::command::export!(scenario);

async fn scenario() {
    assert!(preopens::get_directories().iter().any(|(_, name)| name == "."), "host must mount `.`");

    let error = Request::builder()
        .messages(vec![user("hi")])
        .workspace(".")
        .build()
        .complete()
        .await
        .expect_err("write against a read-only mount fails");
    fails_with(&error, "other", "read-only");
}
