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

use omnia_sdk::plugins::{self, Source};

use crate::omnia_test::link::ops;

omnia_sdk::command::export!(scenario);

async fn scenario() {
    let plugin = plugins::load(Source::Declared("noseam".to_owned()))
        .await
        .expect("the load runs")
        .expect("an unlinked component loads");
    assert_eq!(plugin.name, "noseam");

    // Never returns: the target serves no `ops`, so the link call traps.
    let answer = ops::ping(&plugin.name, "hi");
    panic!("a link call to an unlinked guest must not answer: {answer}");
}
