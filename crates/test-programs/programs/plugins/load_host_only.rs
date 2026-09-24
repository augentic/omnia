//! Plugin-only: load a host-only handler and exit. This guest imports no
//! `ops`, so nothing links it to what it loads; the host drives the loaded
//! echoer through `Dispatcher` after the run.

#![cfg(target_arch = "wasm32")]

use omnia_sdk::plugins::{Location, PluginRef, Plugins as _, WasiPlugins};

omnia_sdk::command!(scenario);

async fn scenario() {
    let plugin = WasiPlugins
        .load(&PluginRef {
            location: Location::Path("./plugin.wasm".to_owned()),
            digest: None,
        })
        .await
        .expect("a host-only handler loads without a linked import");
    assert_eq!(plugin.id(), "plugin");
}
