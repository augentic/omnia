//! `generation`, an MCP grant, and a worker grant cross the boundary intact.

#![cfg(target_arch = "wasm32")]

use omnia_sdk::model::{
    Effort, Generation, McpGrant, Model as _, Request, Tool, WasiModel, WorkerGrant,
};
use test_programs::user;

omnia_sdk::command!(scenario);

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
                .workspace(".")
                .worker(
                    WorkerGrant::builder()
                        .egress(vec!["registry.npmjs.org".to_owned(), "*.cursor.sh".to_owned()])
                        .read(vec!["~/.nvm/versions/node".to_owned()])
                        .env(vec![("VOLTA_HOME".to_owned(), "~/.volta".to_owned())])
                        .build(),
                )
                .build(),
        )
        .await
        .expect("echo answers");
    assert_eq!(reply.answer, "hi");
}
