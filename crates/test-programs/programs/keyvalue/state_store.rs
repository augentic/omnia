//! The `omnia_sdk::contract::state` rows over `WasiProvider`, all at once,
//! so the host's `omnia:keyvalue` meets the contract the native doubles do.
//! Three keys of the program's own are left for the host to read back.

#![cfg(target_arch = "wasm32")]

use std::time::Duration;

use omnia_sdk::contract::state;
use omnia_sdk::{StateStore as _, WasiProvider};

omnia_sdk::api::command::export!(scenario);

async fn scenario() {
    let store = WasiProvider;

    store.set("host.raw", b"raw", Some(Duration::from_hours(1))).await.expect("set with a TTL");
    store.set("host.persist", b"kept", None).await.expect("set without a TTL");
    store.set("host.expiring", b"gone", Some(Duration::from_millis(100))).await.expect("set");

    futures::join!(
        state::persist(&store, wait),
        state::set_ttl(&store, wait),
        state::zero_ttl(&store),
        state::create_ttl(&store, wait),
        state::update_ttl(&store, wait),
        state::swap_expiring(&store),
        state::conflict(&store),
        state::counter(&store),
        state::named(&store),
        state::undeclared(&store),
        state::delete_absent(&store),
        state::portable_key(&store),
        state::malformed_json(&store),
    );
}

async fn wait(duration: Duration) {
    let nanos = u64::try_from(duration.as_nanos()).expect("a row waits less than 584 years");
    wasip3::clocks::monotonic_clock::wait_for(nanos).await;
}
