//! `generation` and an MCP grant cross the boundary intact.

#![cfg(target_arch = "wasm32")]

use omnia_guest::model::{Effort, Generation, McpGrant, Model as _, Request, Tool, WasiModel};
use test_programs::user;

test_programs::run!(scenario);

async fn scenario() {
    let reply = WasiModel
        .complete(
            Request::builder()
                .messages(vec![user("hi")])
                .generation(
                    Generation::builder()
                        .temperature(0.25)
                        .max_tokens(32)
                        .effort(Effort::Low)
                        .build(),
                )
                .tools(vec![Tool::Mcp(
                    McpGrant::builder().name("docs").url("https://mcp.example").build(),
                )])
                .build(),
        )
        .await
        .expect("echo answers");
    assert_eq!(reply.answer, "hi");
}
