//! Raw bindings: a nested directory descriptor is not an authorized mount root.

#![cfg(target_arch = "wasm32")]

use omnia_sdk::model::types;
use omnia_sdk::wit_stream;
use test_programs::{raw_model, raw_request};
use wasip3::filesystem::preopens;
use wasip3::filesystem::types::{DescriptorFlags, OpenFlags, PathFlags};

omnia_sdk::command::export!(scenario);

async fn scenario() {
    let directories = preopens::get_directories();
    let root = directories
        .iter()
        .find(|(_, name)| name == ".")
        .map(|(dir, _)| dir)
        .expect("host mounts `.`");

    let nested = root
        .open_at(
            PathFlags::empty(),
            "nested".to_owned(),
            OpenFlags::DIRECTORY,
            DescriptorFlags::READ,
        )
        .await
        .expect("open nested dir");

    let grants = types::Grants {
        workspace: Some(types::WorkspaceGrant {
            location: types::Location {
                root: &nested,
                subpath: String::new(),
            },
            worker: None,
        }),
    };

    let model = raw_model().await;
    let (results, results_rx) = wit_stream::new();
    let error = model
        .complete(raw_request(vec![], grants), results_rx)
        .await
        .expect_err("nested descriptor is not a mount root");
    drop(results);
    assert!(
        matches!(error, types::Error::Denied(ref detail) if detail.contains("not an authorized mount")),
        "unexpected: {error:?}"
    );
}
