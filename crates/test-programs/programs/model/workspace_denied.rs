//! Without a workspace grant the host-injected tools refuse to run.

#![cfg(target_arch = "wasm32")]

use omnia_guest::model::{Error, Model as _, Request, WasiModel};
use test_programs::user;

test_programs::command!(scenario);

async fn scenario() {
    let error = WasiModel
        .complete(Request::builder().messages(vec![user("hi")]).build())
        .await
        .expect_err("workspace tools require a grant");
    assert!(
        matches!(error, Error::Backend(ref detail) if detail.contains("requires grants.workspace")),
        "unexpected: {error:?}"
    );
}
