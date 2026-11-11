//! A backend call to an undeclared tool fails the completion loud.

#![cfg(target_arch = "wasm32")]

use omnia_sdk::WasiProvider;
use omnia_sdk::model::{Error, McpGrant, Model as _, Request, Tool};
use test_programs::user;

omnia_sdk::api::command::export!(scenario);

async fn scenario() {
    // No function tools declared; the host rejects the backend's `lookup`
    // call before the guest ever sees it. An MCP grant of the same name is
    // the backend's to reach, not a function tool the guest answers.
    let error = WasiProvider
        .complete(
            Request::builder()
                .messages(vec![user("hi")])
                .tools(vec![Tool::Mcp(
                    McpGrant::builder().name("lookup").url("https://mcp.example").build(),
                )])
                .build(),
        )
        .await
        .expect_err("undeclared tool fails the completion");
    assert!(
        matches!(error, Error::ToolFailed(ref detail) if detail.contains("does not declare")),
        "unexpected: {error:?}"
    );
}
