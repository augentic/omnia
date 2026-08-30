//! Happy path through the omnia-guest requester SDK: load a mounted
//! component unpinned (trust-on-first-use — the resolved digest returns),
//! dispatch through the returned handle's identity, then prove idempotency
//! by re-loading pinned with that digest. The loader import arrives through
//! the SDK's own bindings; this world only imports `ops`.

#![cfg(target_arch = "wasm32")]

wit_bindgen::generate!({
    world: "caller",
    path: "wit",
    generate_all,
});

use omnia_guest::plugins::{Location, PluginRef, Plugins as _, WasiPlugins};
use omnia_test::link::ops;

test_programs::run!(scenario);

fn echoer(digest: Option<omnia_guest::plugins::Digest>) -> PluginRef {
    PluginRef::builder()
        .package("test:echoer")
        .location(Location::Path("./plugin.wasm".to_owned()))
        .maybe_digest(digest)
        .build()
}

async fn scenario() {
    let plugin = WasiPlugins.load(&echoer(None)).await.expect("unpinned load succeeds");
    assert_eq!(plugin.id(), "test:echoer");
    // The typed digest is the TOFU report an operator would commit as a pin.
    let digest = plugin.digest().clone();

    // The handle's identity routes host-mediated dispatch to the exporter.
    let answer = ops::ping(plugin.id(), "hi");
    assert_eq!(answer, "test:echoer pong: hi");

    // Second load is idempotent, and the reported digest pins it exactly.
    let again =
        WasiPlugins.load(&echoer(Some(digest.clone()))).await.expect("pinned re-load succeeds");
    assert_eq!(again.id(), plugin.id());
    assert_eq!(again.digest(), &digest);
}
