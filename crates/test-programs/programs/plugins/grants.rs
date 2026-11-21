//! Loads held to the requester's grants. A declared guest they reach loads,
//! and one they do not is denied. A path beneath a mount they reach loads,
//! and one beneath a mount they do not is denied. A component importing a
//! package outside the grants it would run under is denied, naming the
//! import.

#![cfg(target_arch = "wasm32")]

use omnia_sdk::plugins::types::Error;
use omnia_sdk::plugins::{File, Source, loader};

omnia_sdk::command::export!(scenario);

fn path(path: &str) -> Source {
    Source::Path(File {
        path: path.to_owned(),
        pin: None,
    })
}

// the detail of a denied load must carry `needle`
async fn denied(source: Source, needle: &str) {
    let name = source.name().to_owned();
    match loader::load(source).await {
        Err(Error::Denied(detail)) => {
            assert!(detail.contains(needle), "`{detail}` does not mention `{needle}`");
        }
        Ok(_) => panic!("`{name}` loaded, where its load is denied"),
        Err(other) => panic!("`{name}` failed with {other:?}, where its load is denied"),
    }
}

async fn scenario() {
    let plugin = loader::load(Source::Declared("plugin".to_owned())).await;
    let plugin = plugin.expect("the grants reach `plugin`").expect("no conflict");
    assert_eq!(plugin.name, "plugin");
    denied(Source::Declared("secret".to_owned()), "reach no guest `secret`").await;

    let tool = loader::load(path("store/tool.wasm")).await;
    let tool = tool.expect("the grants reach `store`").expect("no conflict");
    assert_eq!(tool.name, "tool");
    denied(path("vault/tool.wasm"), "beneath the mount `vault`").await;
    denied(path("store/full.wasm"), "imports `omnia-test:link/ops").await;
}
