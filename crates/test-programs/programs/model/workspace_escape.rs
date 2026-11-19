//! Raw bindings: a workspace location whose subpath is not a plain relative
//! path beneath the lent root is refused at `complete`.

#![cfg(target_arch = "wasm32")]

use omnia_sdk::model::types;
use omnia_sdk::wit_stream;
use test_programs::{raw_model, raw_request};
use wasip3::filesystem::preopens;

omnia_sdk::command::export!(scenario);

async fn scenario() {
    let directories = preopens::get_directories();
    let root = directories
        .iter()
        .find(|(_, name)| name == ".")
        .map(|(dir, _)| dir)
        .expect("host mounts `.`");

    let model = raw_model().await;
    for subpath in
        ["../escape", "/abs", "docs\\guides", "docs//guides", ".", "..", "a/../b", "./a", "a/"]
    {
        let grants = types::Grants {
            workspace: Some(types::WorkspaceGrant {
                location: types::Location {
                    root,
                    subpath: subpath.to_owned(),
                },
                worker: None,
            }),
        };

        let (results, results_rx) = wit_stream::new();
        let error = model
            .complete(raw_request(vec![], grants), results_rx)
            .await
            .expect_err("subpath refused");
        drop(results);
        assert!(
            matches!(error, types::Error::Invalid(ref detail) if detail.contains("not a plain relative path")),
            "`{subpath}`: unexpected: {error:?}"
        );
    }
}
