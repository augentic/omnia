//! A read-only mount lets the backend read, but write is refused.

#![cfg(target_arch = "wasm32")]

use omnia_guest::model::{Error, Model as _, Request, WasiModel};
use test_programs::user;
use wasip3::filesystem::preopens;

omnia_guest::command!(scenario);

async fn scenario() {
    assert!(preopens::get_directories().iter().any(|(_, name)| name == "."), "host must mount `.`");

    let error = WasiModel
        .complete(Request::builder().messages(vec![user("hi")]).workspace(".").build())
        .await
        .expect_err("write against a read-only mount fails");
    assert!(
        matches!(error, Error::Backend(ref detail) if detail.contains("read-only")),
        "unexpected: {error:?}"
    );
}
