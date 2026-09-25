//! Mixed: load a link target and a host-only handler. Ping the target over
//! the link; the host then drives the handler (which exports no `ops`) after
//! the run. Both loads succeed — admission does not require a linked export.

#![cfg(target_arch = "wasm32")]

wit_bindgen::generate!({
    world: "caller",
    path: "wit",
    generate_all,
});

use omnia_sdk::plugins::{Location, Plugins as _, WasiPlugins};
use omnia_test::link::ops;

omnia_sdk::command!(scenario);

async fn scenario() {
    let echoer = Location::Declared("echoer".to_owned());
    let target = WasiPlugins.load(&echoer, None).await.expect("a link target loads");
    assert_eq!(target.id(), "echoer");

    let answer = ops::ping(target.id(), "hi");
    assert_eq!(answer, "echoer pong: hi");

    let handler = Location::Declared("handler".to_owned());
    let handler = WasiPlugins.load(&handler, None).await.expect("a host-only handler loads");
    assert_eq!(handler.id(), "handler");
}
