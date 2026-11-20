//! The host-injected workspace tools hold a read, a write, and a listing to
//! their caps, failing the completion past each.

#![cfg(target_arch = "wasm32")]

use omnia_sdk::model::Request;
use test_programs::{fails_with, user};
use wasip3::filesystem::preopens;

omnia_sdk::command::export!(scenario);

async fn scenario() {
    assert!(preopens::get_directories().iter().any(|(_, name)| name == "."), "host must mount `.`");

    for needle in
        ["exceeds workspace read limit", "write limit exceeded", "directory exceeds listing limit"]
    {
        let error = Request::builder()
            .messages(vec![user("workspace")])
            .workspace(".")
            .build()
            .complete()
            .await
            .expect_err("a capped workspace step fails the completion");
        fails_with(&error, "other", needle);
    }
}
