//! `generation`, a tool, and a worker grant cross the boundary intact. An
//! effort level the host does not know reaches the backend by its name.

#![cfg(target_arch = "wasm32")]

use omnia_sdk::model::{Effort, Generation, Request, Tool, WorkerGrant};
use test_programs::user;

omnia_sdk::command::export!(scenario);

async fn scenario() {
    let reply = Request::builder()
        .messages(vec![user("hi")])
        .generation(Generation {
            temperature: Some(0.25),
            max_tokens: Some(32),
            effort: Some(Effort::Other("max".to_owned())),
            ..Generation::default()
        })
        .tools(vec![Tool {
            name: "docs".to_owned(),
            description: Some("search the docs".to_owned()),
            input_schema: "{}".to_owned(),
        }])
        .workspace(".")
        .worker(WorkerGrant {
            egress: vec!["registry.npmjs.org".to_owned(), "*.cursor.sh".to_owned()],
            read: vec!["~/.nvm/versions/node".to_owned()],
            env: vec![("VOLTA_HOME".to_owned(), "~/.volta".to_owned())],
        })
        .build()
        .complete()
        .await
        .expect("the completion runs")
        .expect("echo answers");
    assert_eq!(reply.answer, "hi");
}
