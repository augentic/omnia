//! `generation`, a function tool, and a worker grant cross the boundary
//! intact. An effort level the host does not know reaches the backend by its
//! name.

#![cfg(target_arch = "wasm32")]

use omnia_sdk::WasiProvider;
use omnia_sdk::model::{Effort, Function, Generation, Model as _, Request, Tool, WorkerGrant};
use test_programs::user;

omnia_sdk::api::command::export!(scenario);

async fn scenario() {
    let reply = WasiProvider
        .complete(
            Request::builder()
                .messages(vec![user("hi")])
                .generation(
                    Generation::builder()
                        .temperature(0.25)
                        .max_tokens(32)
                        .effort(Effort::Other("max".to_owned()))
                        .build(),
                )
                .tools(vec![Tool::Function(
                    Function::builder()
                        .name("docs")
                        .description("search the docs")
                        .parameters("{}")
                        .build(),
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
