//! Raw bindings: a workspace grant whose subpath is not a plain relative
//! path beneath the lent root is refused at `create`.

#![cfg(target_arch = "wasm32")]

use omnia_wasi_model::{completion, wit_stream};
use test_programs::raw_request;
use wasip3::filesystem::preopens;

omnia_sdk::api::command::export!(scenario);

async fn scenario() {
    let directories = preopens::get_directories();
    let root = directories
        .iter()
        .find(|(_, name)| name == ".")
        .map(|(dir, _)| dir)
        .expect("host mounts `.`");

    for subpath in
        ["../escape", "/abs", "docs\\guides", "docs//guides", ".", "..", "a/../b", "./a", "a/"]
    {
        let grants = completion::Grants {
            workspace: Some(completion::WorkspaceGrant {
                root,
                subpath: subpath.to_owned(),
            }),
            worker: None,
        };

        let (results, results_rx) = wit_stream::new();
        let error = completion::create(raw_request(vec![], grants), results_rx)
            .await
            .expect_err("subpath refused");
        drop(results);
        assert!(
            matches!(error, completion::Error::Backend(ref detail) if detail.contains("not a plain relative path")),
            "`{subpath}`: unexpected: {error:?}"
        );
    }
}
