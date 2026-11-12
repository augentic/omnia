//! Config lookups answer from the host's seeded map and nothing else: the
//! seeded key resolves, a variable every test process carries in its
//! environment reads as unset, and `get-all` enumerates exactly the seeded map.

#![cfg(target_arch = "wasm32")]

use omnia_sdk::{Config, WasiProvider};

omnia_sdk::api::command::export!(scenario);

async fn scenario() {
    let greeting = WasiProvider.get("GREETING").await.expect("seeded key");
    assert_eq!(greeting.as_deref(), Some("hello"));
    let path = WasiProvider.get("PATH").await.expect("an unset key is not an error");
    assert_eq!(path, None, "the process environment does not leak");

    // `omnia_sdk::Config` has no `get-all`; go through the raw binding.
    let all = omnia_wasi_config::store::get_all().expect("get-all");
    assert_eq!(all, [("GREETING".to_owned(), "hello".to_owned())]);
}
