//! The `omnia_test::contract::state` rows over the `default` bucket, all at
//! once, so the host's `omnia:keyvalue` and its backend meet the contract.
//! Three keys of the program's own are left for the host to read back.

#![cfg(target_arch = "wasm32")]

use std::time::Duration;

use omnia_sdk::keyvalue::bucket::Bucket;
use omnia_sdk::keyvalue::types::Lifetime;
use omnia_sdk::keyvalue::{DEFAULT, store};
use omnia_test::contract::state;

omnia_sdk::api::command::export!(scenario);

async fn scenario() {
    let bucket = store::open(DEFAULT.to_owned()).await.expect("open the default bucket");

    leave(&bucket, "host.raw", b"raw", Duration::from_hours(1).into()).await;
    leave(&bucket, "host.persist", b"kept", Lifetime::Forever).await;
    leave(&bucket, "host.expiring", b"gone", Duration::from_millis(100).into()).await;

    futures::join!(
        state::persist(&bucket, wait),
        state::set_ttl(&bucket, wait),
        state::zero_ttl(&bucket),
        state::create_ttl(&bucket, wait),
        state::update_ttl(&bucket, wait),
        state::swap_expiring(&bucket),
        state::conflict(&bucket),
        state::revision(&bucket),
        state::counter(&bucket),
        state::named(&bucket),
        state::undeclared(),
        state::delete_absent(&bucket),
        state::portable_key(&bucket),
        state::json(&bucket),
        state::malformed_json(&bucket),
    );
}

async fn leave(bucket: &Bucket, key: &str, value: &[u8], lifetime: Lifetime) {
    let written = bucket.set(key.to_owned(), value.to_vec(), lifetime, None).await;
    written.expect("set").expect("an unconditional set conflicted");
}

async fn wait(duration: Duration) {
    let nanos = u64::try_from(duration.as_nanos()).expect("a row waits less than 584 years");
    wasip3::clocks::monotonic_clock::wait_for(nanos).await;
}
