//! Happy path through the omnia-sdk plugin requester: load a mounted
//! component unpinned (trust-on-first-use — the resolved digest returns),
//! dispatch through the returned handle's identity — the path's file stem —
//! then prove idempotency by re-loading pinned with that digest. The loader
//! import arrives through the SDK's own bindings; this world only imports
//! `ops`.

#![cfg(target_arch = "wasm32")]

wit_bindgen::generate!({
    world: "caller",
    path: "wit",
    generate_all,
});

use omnia_sdk::plugins::{Location, PluginRef, Plugins as _, WasiPlugins};
use omnia_test::link::ops;

omnia_sdk::command!(scenario);

fn echoer(digest: Option<omnia_sdk::plugins::Digest>) -> PluginRef {
    PluginRef {
        location: Location::Path("./plugin.wasm".to_owned()),
        digest,
    }
}

async fn scenario() {
    let plugin = WasiPlugins.load(&echoer(None)).await.expect("unpinned load succeeds");
    assert_eq!(plugin.id(), "plugin", "a path load registers as its file stem");
    // The typed digest is the TOFU report an operator would commit as a pin.
    let digest = plugin.digest().cloned().expect("a loaded component reports its digest");

    // The handle's identity routes host-mediated dispatch to the exporter.
    let answer = ops::ping(plugin.id(), "hi");
    assert_eq!(answer, "plugin pong: hi");

    // Second load is idempotent, and the reported digest pins it exactly.
    let again =
        WasiPlugins.load(&echoer(Some(digest.clone()))).await.expect("pinned re-load succeeds");
    assert_eq!(again.id(), plugin.id());
    assert_eq!(again.digest(), Some(&digest));
}
