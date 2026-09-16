//! Config lookups answer from the host's seeded map and nothing else: the
//! seeded key resolves, a variable every test process carries in its
//! environment does not, and `get-all` enumerates exactly the seeded map.

#![cfg(target_arch = "wasm32")]

use omnia_sdk::Config;

omnia_sdk::command!(scenario);

struct WasiConfig;

impl Config for WasiConfig {}

async fn scenario() {
    assert_eq!(WasiConfig.get("GREETING").await.expect("seeded key"), "hello");
    assert!(WasiConfig.get("PATH").await.is_err(), "the process environment does not leak");

    // `omnia_sdk::Config` has no `get-all`; go through the raw binding.
    let all = omnia_wasi_config::store::get_all().expect("get-all");
    assert_eq!(all, [("GREETING".to_owned(), "hello".to_owned())]);
}
