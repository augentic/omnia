//! Mixed: load a link target and a host-only handler. Ping the target over
//! the link; the host then drives the handler (which exports no `ops`) after
//! the run. Both loads succeed — admission does not require a linked export.

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
    let target = plugins::load(Source::Declared("echoer".to_owned()))
        .await
        .expect("the load runs")
        .expect("a link target loads");
    assert_eq!(target.name, "echoer");

    let answer = ops::ping(&target.name, "hi");
    assert_eq!(answer, "echoer pong: hi");

    let handler = plugins::load(Source::Declared("handler".to_owned()))
        .await
        .expect("the load runs")
        .expect("a host-only handler loads");
    assert_eq!(handler.name, "handler");
}
