//! A component exporting nothing outside the host's namespaces still loads:
//! admission does not require a linked export, because such a guest remains
//! reachable through the host `Dispatcher`. Dispatching to it over the link
//! instead fails at the call site — the polyfill traps this caller, so the
//! host side asserts the run fails after the load succeeded.

#![cfg(target_arch = "wasm32")]

wit_bindgen::generate!({
    world: "caller",
    path: "wit",
    generate_all,
});

use omnia_sdk::plugins::{Plugins as _, WasiPlugins};
use omnia_test::link::ops;

omnia_sdk::command!(scenario);

async fn scenario() {
    let plugin = WasiPlugins.load("noseam").await.expect("an unlinked component loads");
    assert_eq!(plugin.id(), "noseam");

    // Never returns: the target serves no `ops`, so the link call traps.
    let answer = ops::ping(plugin.id(), "hi");
    panic!("a link call to an unlinked guest must not answer: {answer}");
}
