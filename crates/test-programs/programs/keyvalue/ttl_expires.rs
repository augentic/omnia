//! Implementation-enforced TTL over `omnia:keyvalue/ttl`: a key written with
//! `ttl::set` is stored as raw bytes, survives a compare-and-swap with its
//! remaining TTL intact, and reads as absent once the TTL elapses.

#![cfg(target_arch = "wasm32")]

use std::time::Duration;

use omnia_wasi_keyvalue::atomics::{self, Cas};
use omnia_wasi_keyvalue::{store, ttl};

omnia_sdk::api::command::export!(scenario);

async fn scenario() {
    let bucket = store::open("bucket".to_owned()).await.expect("open");

    ttl::set(&bucket, "ephemeral".to_owned(), b"v1".to_vec(), 1).await.expect("set ephemeral");
    ttl::set(&bucket, "live".to_owned(), b"raw-bytes".to_vec(), 60).await.expect("set live");

    assert_eq!(bucket.get("ephemeral".to_owned()).await.expect("get"), Some(b"v1".to_vec()));
    assert!(bucket.exists("ephemeral".to_owned()).await.expect("exists"));

    // CAS updates the bytes and preserves the remaining TTL rather than clearing it.
    let cas = Cas::new(&bucket, "ephemeral".to_owned()).await.expect("cas");
    assert_eq!(cas.current().await.expect("current"), Some(b"v1".to_vec()));
    atomics::swap(cas, b"v2".to_vec()).await.expect("swap");
    assert_eq!(bucket.get("ephemeral".to_owned()).await.expect("get"), Some(b"v2".to_vec()));

    std::thread::sleep(Duration::from_millis(1100));

    assert_eq!(bucket.get("ephemeral".to_owned()).await.expect("get"), None);
    assert!(!bucket.exists("ephemeral".to_owned()).await.expect("exists"));
    assert_eq!(bucket.get("live".to_owned()).await.expect("get"), Some(b"raw-bytes".to_vec()));
}
