//! A backend call to an undeclared tool fails the completion loud.

#![cfg(target_arch = "wasm32")]

use omnia_sdk::WasiProvider;
use omnia_sdk::model::{Error, Model as _, Request};
use test_programs::user;

omnia_sdk::export!(command: scenario);

async fn scenario() {
    // No tools declared; the host rejects the backend's `lookup` call
    // before the guest ever sees it.
    let error = WasiProvider
        .complete(Request::builder().messages(vec![user("hi")]).build())
        .await
        .expect_err("undeclared tool fails the completion");
    assert!(
        matches!(error, Error::ToolFailed(ref detail) if detail.contains("does not declare")),
        "unexpected: {error:?}"
    );
}
