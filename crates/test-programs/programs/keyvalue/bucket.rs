//! One bucket walked end to end over the raw `wasi:keyvalue` bindings: an
//! atomic increment lands on the host-seeded counter, the store methods
//! round-trip one key, the batch functions align positionally with a missing
//! key, `list-keys` is a single page, and a compare-and-swap succeeds on a
//! fresh snapshot then fails on a stale one with a live refreshed handle.

#![cfg(target_arch = "wasm32")]

use omnia_wasi_keyvalue::atomics::{self, Cas, CasError};
use omnia_wasi_keyvalue::{batch, store};

omnia_guest::command!(scenario);

async fn scenario() {
    let bucket = store::open("bucket".to_owned()).await.expect("open");

    // Atomics: the host seeded `counter` as big-endian 37; an absent key
    // starts from zero.
    assert_eq!(atomics::increment(&bucket, "counter".to_owned(), 5).await.expect("increment"), 42);
    assert_eq!(atomics::increment(&bucket, "fresh".to_owned(), 3).await.expect("increment"), 3);

    // Store: set, read back, probe, delete.
    bucket.set("k".to_owned(), b"v".to_vec()).await.expect("set");
    assert_eq!(bucket.get("k".to_owned()).await.expect("get"), Some(b"v".to_vec()));
    assert!(bucket.exists("k".to_owned()).await.expect("exists"));
    assert!(!bucket.exists("missing".to_owned()).await.expect("exists"));
    bucket.delete("k".to_owned()).await.expect("delete");
    assert_eq!(bucket.get("k".to_owned()).await.expect("get"), None);
    assert!(!bucket.exists("k".to_owned()).await.expect("exists"));

    // Batch: one entry per requested key, `None` where the key is absent.
    let pairs = vec![("a".to_owned(), b"1".to_vec()), ("b".to_owned(), b"2".to_vec())];
    batch::set_many(&bucket, pairs).await.expect("set-many");
    let many = batch::get_many(&bucket, vec!["a".to_owned(), "missing".to_owned(), "b".to_owned()])
        .await
        .expect("get-many");
    assert_eq!(
        many,
        [Some(("a".to_owned(), b"1".to_vec())), None, Some(("b".to_owned(), b"2".to_vec()))]
    );

    // The default returns every key in one page; the deleted `k` is gone.
    let page = bucket.list_keys(None).await.expect("list-keys");
    let mut keys = page.keys;
    keys.sort();
    assert_eq!(keys, ["a", "b", "counter", "fresh"]);
    assert_eq!(page.cursor, None);

    batch::delete_many(&bucket, vec!["a".to_owned(), "missing".to_owned()])
        .await
        .expect("delete-many");
    let many =
        batch::get_many(&bucket, vec!["a".to_owned(), "b".to_owned()]).await.expect("get-many");
    assert_eq!(many, [None, Some(("b".to_owned(), b"2".to_vec()))]);

    // CAS: a swap against an unchanged snapshot succeeds.
    bucket.set("cas".to_owned(), b"v1".to_vec()).await.expect("set");
    let cas = Cas::new(&bucket, "cas".to_owned()).await.expect("cas");
    assert_eq!(cas.current().await.expect("current"), Some(b"v1".to_vec()));
    atomics::swap(cas, b"v2".to_vec()).await.expect("swap on a fresh snapshot");
    assert_eq!(bucket.get("cas".to_owned()).await.expect("get"), Some(b"v2".to_vec()));

    // A write slipping in between `new` and `swap` fails the swap and hands
    // back a handle refreshed at the observed value, which then swaps.
    let cas = Cas::new(&bucket, "cas".to_owned()).await.expect("cas");
    assert_eq!(cas.current().await.expect("current"), Some(b"v2".to_vec()));
    bucket.set("cas".to_owned(), b"interfering".to_vec()).await.expect("set");
    let fresh = match atomics::swap(cas, b"lost-race".to_vec()).await {
        Err(CasError::CasFailed(fresh)) => fresh,
        Ok(()) => panic!("stale swap succeeded"),
        Err(CasError::StoreError(error)) => panic!("stale swap failed with {error:?}"),
    };
    assert_eq!(fresh.current().await.expect("current"), Some(b"interfering".to_vec()));
    atomics::swap(fresh, b"retried".to_vec()).await.expect("swap with the refreshed handle");
    assert_eq!(bucket.get("cas".to_owned()).await.expect("get"), Some(b"retried".to_vec()));
}
