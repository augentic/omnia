//! The [`StateStore`] rows.
//!
//! Each row writes keys of its own, so the rows run in any order, or all at
//! once, over one store. Every key is portable, as [`StateStore`] defines
//! it. The store must:
//!
//! - declare its own bucket and [`SECOND`], both empty when the rows start;
//! - leave [`UNDECLARED`] undeclared.
//!
//! A row that waits out a TTL takes a `wait` that returns once the duration
//! has passed on the store's clock.

use std::time::Duration;

use bytes::Bytes;
use omnia_sdk::capability::ErrorKind;
use omnia_sdk::keyvalue::{Conflict, DEFAULT};
use omnia_sdk::{StateStore, StateStoreExt};

use super::expect_kind;

/// A bucket the store declares beside its own.
pub const SECOND: &str = "second";

/// A bucket the store does not declare.
pub const UNDECLARED: &str = "undeclared";

// long enough that no pause between two calls outlasts it
const TTL: Duration = Duration::from_secs(1);
const EXPIRED: Duration = Duration::from_secs(2);
const LONG: Duration = Duration::from_hours(1);

// every kind of character a portable key may hold
const PORTABLE: &str = "Portable-key_09.end";

/// A value set without a TTL outlasts every TTL the rows use.
///
/// # Panics
///
/// Panics if the value is gone after the wait.
pub async fn persist(store: &impl StateStore, wait: impl AsyncFn(Duration)) {
    store.set("persist", b"kept", None).await.expect("set without a TTL");

    wait(EXPIRED).await;

    assert_eq!(
        value(store, "persist").await,
        Some(b"kept".to_vec()),
        "a value without a TTL expired"
    );
}

/// [`set`](StateStore::set) replaces a value's lifetime along with its bytes.
///
/// # Panics
///
/// Panics if a value keeps the lifetime it was first set with.
pub async fn set_ttl(store: &impl StateStore, wait: impl AsyncFn(Duration)) {
    store.set("set_ttl.expiring", b"one", None).await.expect("set without a TTL");
    store.set("set_ttl.expiring", b"two", Some(TTL)).await.expect("set with a TTL");
    store.set("set_ttl.kept", b"one", Some(TTL)).await.expect("set with a TTL");
    store.set("set_ttl.kept", b"two", None).await.expect("set without a TTL");

    wait(EXPIRED).await;

    assert_eq!(value(store, "set_ttl.expiring").await, None, "a set kept the old lifetime");
    assert_eq!(value(store, "set_ttl.kept").await, Some(b"two".to_vec()), "a set kept the old TTL");
}

/// A zero TTL fails with [`ErrorKind::Invalid`] on every method that takes
/// one, and writes nothing.
///
/// # Panics
///
/// Panics if a zero-TTL call succeeds, fails with another kind, or writes.
pub async fn zero_ttl(store: &impl StateStore) {
    let zero = Some(Duration::ZERO);
    store.set("zero_ttl", b"old", None).await.expect("set without a TTL");

    let set = store.set("zero_ttl", b"new", zero).await;
    let swap = store.compare_and_swap("zero_ttl", Some(b"old"), b"new", zero).await;
    let increment = store.increment("zero_ttl.counter", 1, zero).await;

    expect_kind(set, ErrorKind::Invalid, "set with a zero TTL");
    expect_kind(swap, ErrorKind::Invalid, "compare_and_swap with a zero TTL");
    expect_kind(increment, ErrorKind::Invalid, "increment with a zero TTL");
    assert_eq!(value(store, "zero_ttl").await, Some(b"old".to_vec()), "a zero TTL wrote the value");
    assert_eq!(counter_at(store, "zero_ttl.counter").await, 0, "a zero TTL wrote the counter");
}

/// [`compare_and_swap`](StateStore::compare_and_swap) and
/// [`increment`](StateStore::increment) apply their TTL to an entry they
/// create.
///
/// # Panics
///
/// Panics if a created entry outlives its TTL.
pub async fn create_ttl(store: &impl StateStore, wait: impl AsyncFn(Duration)) {
    let swapped = store.compare_and_swap("create_ttl.lock", None, b"held", Some(TTL)).await;
    let counted = store.increment("create_ttl.counter", 1, Some(TTL)).await;
    assert_eq!(swapped.expect("compare_and_swap"), Ok(()), "a creating swap conflicted");
    assert_eq!(counted.expect("increment"), 1);

    wait(EXPIRED).await;

    assert_eq!(value(store, "create_ttl.lock").await, None, "a created lock outlived its TTL");
    assert_eq!(counter_at(store, "create_ttl.counter").await, 0, "a counter outlived its TTL");
}

/// An update through [`compare_and_swap`](StateStore::compare_and_swap) or
/// [`increment`](StateStore::increment) keeps the time the entry has left.
///
/// # Panics
///
/// Panics if an update refreshes, extends, or clears the entry's lifetime.
pub async fn update_ttl(store: &impl StateStore, wait: impl AsyncFn(Duration)) {
    store.set("update_ttl.value", b"one", Some(TTL)).await.expect("set with a TTL");
    let swapped =
        store.compare_and_swap("update_ttl.value", Some(b"one"), b"two", Some(LONG)).await;
    assert_eq!(swapped.expect("compare_and_swap"), Ok(()), "a matching swap conflicted");
    assert_eq!(store.increment("update_ttl.counter", 1, Some(TTL)).await.expect("increment"), 1);
    assert_eq!(store.increment("update_ttl.counter", 1, Some(LONG)).await.expect("increment"), 2);
    assert_eq!(store.increment("update_ttl.counter", 1, None).await.expect("increment"), 3);

    wait(EXPIRED).await;

    assert_eq!(value(store, "update_ttl.value").await, None, "a swap refreshed the lifetime");
    assert_eq!(counter_at(store, "update_ttl.counter").await, 0, "an increment refreshed it");
}

/// [`compare_and_swap`](StateStore::compare_and_swap) matches the bytes of a
/// value set with a TTL.
///
/// # Panics
///
/// Panics if the swap conflicts or does not land.
pub async fn swap_expiring(store: &impl StateStore) {
    store.set("swap_expiring", b"one", Some(LONG)).await.expect("set with a TTL");

    let swapped = store.compare_and_swap("swap_expiring", Some(b"one"), b"two", None).await;

    assert_eq!(swapped.expect("compare_and_swap"), Ok(()), "a TTL hid the stored bytes");
    assert_eq!(value(store, "swap_expiring").await, Some(b"two".to_vec()));
}

/// A [`compare_and_swap`](StateStore::compare_and_swap) that does not match
/// changes nothing and returns the value it found.
///
/// # Panics
///
/// Panics if a mismatched swap writes or reports another value.
pub async fn conflict(store: &impl StateStore) {
    let absent = store.compare_and_swap("conflict", Some(b"one"), b"two", None).await;
    assert_eq!(absent.expect("compare_and_swap"), Err(Conflict { current: None }));
    assert_eq!(value(store, "conflict").await, None, "a conflicting swap wrote");

    store.set("conflict", b"one", None).await.expect("set without a TTL");
    let present = store.compare_and_swap("conflict", None, b"two", None).await;
    let stale = store.compare_and_swap("conflict", Some(b"zero"), b"two", None).await;

    let found = Conflict {
        current: Some(Bytes::from_static(b"one")),
    };
    assert_eq!(present.expect("compare_and_swap"), Err(found.clone()));
    assert_eq!(stale.expect("compare_and_swap"), Err(found));
    assert_eq!(value(store, "conflict").await, Some(b"one".to_vec()), "a conflicting swap wrote");
}

/// [`increment`](StateStore::increment) starts an absent counter at zero, and
/// a `delta` of zero reads it.
///
/// # Panics
///
/// Panics if a counter starts elsewhere or reads back another value.
pub async fn counter(store: &impl StateStore) {
    assert_eq!(store.increment("counter", 5, None).await.expect("increment"), 5);
    assert_eq!(store.increment("counter", -7, None).await.expect("increment"), -2);
    assert_eq!(counter_at(store, "counter").await, -2);
}

/// Two buckets never share a key, and a store's own handle uses [`DEFAULT`].
///
/// # Panics
///
/// Panics if a write through one name reaches another.
pub async fn named(store: &impl StateStore) {
    let second = store.named(SECOND);
    store.set("named", b"own", None).await.expect("set");
    assert_eq!(value(&second, "named").await, None, "a second bucket saw the store's own key");

    second.set("named", b"second", None).await.expect("set");

    assert_eq!(
        value(store, "named").await,
        Some(b"own".to_vec()),
        "a second bucket overwrote the key"
    );
    assert_eq!(value(&second, "named").await, Some(b"second".to_vec()));
    let default = store.named(DEFAULT);
    assert_eq!(
        value(&default, "named").await,
        Some(b"own".to_vec()),
        "the store's own bucket is not `default`"
    );
}

/// Every call on a bucket the deployment does not declare fails with
/// [`ErrorKind::NotFound`].
///
/// # Panics
///
/// Panics if a call on [`UNDECLARED`] succeeds or fails with another kind.
pub async fn undeclared(store: &impl StateStore) {
    let undeclared = store.named(UNDECLARED);

    let get = undeclared.get("undeclared").await;
    let set = undeclared.set("undeclared", b"v", None).await;
    let delete = undeclared.delete("undeclared").await;
    let swap = undeclared.compare_and_swap("undeclared", None, b"v", None).await;
    let increment = undeclared.increment("undeclared", 1, None).await;

    expect_kind(get, ErrorKind::NotFound, "get on an undeclared bucket");
    expect_kind(set, ErrorKind::NotFound, "set on an undeclared bucket");
    expect_kind(delete, ErrorKind::NotFound, "delete on an undeclared bucket");
    expect_kind(swap, ErrorKind::NotFound, "compare_and_swap on an undeclared bucket");
    expect_kind(increment, ErrorKind::NotFound, "increment on an undeclared bucket");
}

/// Deleting an absent key succeeds.
///
/// # Panics
///
/// Panics if a delete fails or leaves the value behind.
pub async fn delete_absent(store: &impl StateStore) {
    store.delete("delete_absent").await.expect("delete an absent key");
    store.set("delete_absent", b"v", None).await.expect("set");
    store.delete("delete_absent").await.expect("delete");
    store.delete("delete_absent").await.expect("delete the key again");

    assert_eq!(value(store, "delete_absent").await, None);
}

/// A key holding every kind of character a portable key may use works like
/// any other.
///
/// # Panics
///
/// Panics if a call on the key fails or reads back another value.
pub async fn portable_key(store: &impl StateStore) {
    store.set(PORTABLE, b"one", None).await.expect("set a portable key");
    let swapped = store.compare_and_swap(PORTABLE, Some(b"one"), b"two", None).await;
    assert_eq!(swapped.expect("compare_and_swap"), Ok(()), "a matching swap conflicted");
    assert_eq!(value(store, PORTABLE).await, Some(b"two".to_vec()));

    store.delete(PORTABLE).await.expect("delete a portable key");

    assert_eq!(value(store, PORTABLE).await, None, "a delete left the portable key");
}

/// [`get_json`](StateStoreExt::get_json) over bytes that do not decode fails
/// with [`ErrorKind::Invalid`].
///
/// # Panics
///
/// Panics if the read succeeds or fails with another kind.
pub async fn malformed_json(store: &impl StateStore) {
    store.set("malformed_json", b"{not json", None).await.expect("set");

    let decoded = store.get_json::<serde_json::Value>("malformed_json").await;

    expect_kind(decoded, ErrorKind::Invalid, "get_json over malformed bytes");
}

async fn value(store: &impl StateStore, key: &str) -> Option<Vec<u8>> {
    store.get(key).await.unwrap_or_else(|err| panic!("get `{key}`: {err}")).map(Vec::from)
}

async fn counter_at(store: &impl StateStore, key: &str) -> i64 {
    store.increment(key, 0, None).await.unwrap_or_else(|err| panic!("read `{key}`: {err}"))
}
