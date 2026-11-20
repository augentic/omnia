//! Plugin-only: load a host-only handler and exit. This guest imports no
//! `ops`, so nothing links it to what it loads; the host drives the loaded
//! echoer through `Dispatcher` after the run.

#![cfg(target_arch = "wasm32")]

use omnia_sdk::plugins::{self, Source};

omnia_sdk::command::export!(scenario);

async fn scenario() {
    let plugin = plugins::load(Source::Declared("plugin".to_owned()))
        .await
        .expect("the load runs")
        .expect("a host-only handler loads without a linked import");
    assert_eq!(plugin.name, "plugin");
}
