//! Happy path through a registry location: load the versioned package from
//! the deployment's default registry, dispatch through the returned handle's
//! identity — the package reference — then prove idempotency by re-loading
//! pinned with the resolved digest. The loader import arrives through the
//! SDK's own bindings; this world only imports `ops`.

#![cfg(target_arch = "wasm32")]

wit_bindgen::generate!({
    world: "caller",
    path: "wit",
    generate_all,
});

use omnia_sdk::plugins::{Location, PluginRef, Plugins as _, WasiPlugins};
use omnia_test::link::ops;

omnia_sdk::command!(scenario);

const PACKAGE: &str = "test:echoer@1.0.0";

fn echoer(digest: Option<omnia_sdk::plugins::Digest>) -> PluginRef {
    PluginRef {
        location: Location::Registry {
            package: PACKAGE.to_owned(),
            endpoint: None,
        },
        digest,
    }
}

async fn scenario() {
    let plugin = WasiPlugins.load(&echoer(None)).await.expect("registry load succeeds");
    assert_eq!(plugin.id(), PACKAGE, "a registry load registers as its package reference");

    // The handle's identity routes host-mediated dispatch to the exporter.
    let answer = ops::ping(plugin.id(), "hi");
    assert_eq!(answer, format!("{PACKAGE} pong: hi"));

    // Second load pinned to the resolved digest is idempotent.
    let digest = plugin.digest().cloned().expect("a loaded component reports its digest");
    let again =
        WasiPlugins.load(&echoer(Some(digest.clone()))).await.expect("pinned re-load succeeds");
    assert_eq!(again.id(), plugin.id());
    assert_eq!(again.digest(), Some(&digest));
}
