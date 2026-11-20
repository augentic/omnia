//! `Bucket::update_json` raced by a write that lands between its read and
//! its store: the closure runs again over the value that write stored, and
//! the update lands on top of it.

#![cfg(target_arch = "wasm32")]

use omnia_sdk::keyvalue;

omnia_sdk::command::export!(scenario);

async fn scenario() {
    let bucket = keyvalue::default().await.expect("open the default bucket");

    let mut seen = Vec::new();
    let stored = bucket.update_json("visits", None, |visits: Option<u32>| {
        seen.push(visits);
        Ok(visits.unwrap_or_default() + 1)
    });
    let stored = stored.await.expect("update_json");

    // the first run read the host's seed, and the second the racing write
    assert_eq!(seen, [Some(1), Some(10)]);
    assert_eq!(stored, 11);
}
