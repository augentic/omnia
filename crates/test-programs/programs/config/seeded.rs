//! Config lookups answer from the host's seeded map and nothing else: the
//! seeded key resolves, and a variable every test process carries in its
//! environment does not.

#![cfg(target_arch = "wasm32")]

use omnia_guest::Config;

omnia_guest::command!(scenario);

struct WasiConfig;

impl Config for WasiConfig {}

async fn scenario() {
    assert_eq!(WasiConfig.get("GREETING").await.expect("seeded key"), "hello");
    assert!(WasiConfig.get("PATH").await.is_err(), "the process environment does not leak");
}
