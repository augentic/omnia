//! Without a workspace grant the host-injected tools refuse to run.

#![cfg(target_arch = "wasm32")]

use omnia_sdk::WasiProvider;
use omnia_sdk::model::{Error, Model as _, Request};
use test_programs::user;

omnia_sdk::export!(command: scenario);

async fn scenario() {
    let error = WasiProvider
        .complete(Request::builder().messages(vec![user("hi")]).build())
        .await
        .expect_err("workspace tools require a grant");
    assert!(
        matches!(error, Error::Backend(ref detail) if detail.contains("requires grants.workspace")),
        "unexpected: {error:?}"
    );
}
