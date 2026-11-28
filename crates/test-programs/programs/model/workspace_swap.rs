//! Rearranging the mount while a completion holds a lend of it.
//!
//! - The guest lends `./tree` and declares a `swap` tool.
//! - The handler moves `tree` aside to `held`, then lays `tree` as a
//!   relative link that climbs out of the mount to `escape`.
//! - The backend answers once the swap is done.

#![cfg(target_arch = "wasm32")]

use omnia_sdk::model::{Request, Tool};
use test_programs::user;
use wasip3::filesystem::preopens;

omnia_sdk::command::export!(scenario);

async fn scenario() {
    let directories = preopens::get_directories();
    let root = directories
        .iter()
        .find(|(_, name)| name == ".")
        .map(|(dir, _)| dir)
        .expect("host must mount `.`");

    let request = Request::builder()
        .messages(vec![user("swap")])
        .tools(vec![swap()])
        .workspace("./tree")
        .build();
    let reply = request
        .complete_with(|call| async move {
            assert_eq!(call.name, "swap");
            root.rename_at("tree".to_owned(), root, "held".to_owned())
                .await
                .expect("move the lent tree aside");
            root.symlink_at("../escape".to_owned(), "tree".to_owned())
                .await
                .expect("link the lent name out of the mount");
            Ok::<_, String>("swapped".to_owned())
        })
        .await
        .expect("the completion runs")
        .expect("the backend answers after the swap");
    assert_eq!(reply.answer, "done");
}

fn swap() -> Tool {
    Tool {
        name: "swap".to_owned(),
        description: Some("rearrange the mount".to_owned()),
        input_schema: "{}".to_owned(),
    }
}
