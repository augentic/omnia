//! The `omnia:keyvalue` rows, over a [`Bucket`].
//!
//! Each row writes keys of its own, so the rows run in any order, or all at
//! once, over one bucket. Every key is portable, as `omnia:keyvalue`
//! defines it. The deployment must:
//!
//! - declare [`DEFAULT`], the rows' bucket, and [`SECOND`], both empty when
//!   the rows start;
//! - leave [`UNDECLARED`] undeclared.
//!
//! A row that waits out a TTL takes a `wait` that returns once the duration
//! has passed on the host's clock.

use std::time::Duration;

use omnia_sdk::keyvalue::{
    self, Bucket, Conflict, DEFAULT, Entry, Lifetime, Precondition, Revision, ttl_ms,
};

use super::expect_code;

/// A bucket the deployment declares beside [`DEFAULT`].
pub const SECOND: &str = "second";

/// A bucket the deployment does not declare.
pub const UNDECLARED: &str = "undeclared";

// long enough that no pause between two calls outlasts it
const TTL: Duration = Duration::from_secs(1);
const EXPIRED: Duration = Duration::from_secs(2);
const LONG: Duration = Duration::from_hours(1);

// every kind of character a portable key may hold
const PORTABLE: &str = "Portable-key_09.end";

/// A value written [`Forever`](Lifetime::Forever) outlasts every TTL the
/// rows use.
///
/// # Panics
///
/// Panics if the value is gone after the wait.
pub async fn persist(bucket: &Bucket, wait: impl AsyncFn(Duration)) {
    write(bucket, "persist", b"kept", Lifetime::Forever).await;

    wait(EXPIRED).await;

    assert_eq!(
        value(bucket, "persist").await,
        Some(b"kept".to_vec()),
        "a value without a TTL expired"
    );
}

/// An unconditional [`set`](Bucket::set) replaces a value's lifetime along
/// with its bytes.
///
/// # Panics
///
/// Panics if a value keeps the lifetime it was first set with.
pub async fn set_ttl(bucket: &Bucket, wait: impl AsyncFn(Duration)) {
    write(bucket, "set_ttl.expiring", b"one", Lifetime::Forever).await;
    write(bucket, "set_ttl.expiring", b"two", TTL.into()).await;
    write(bucket, "set_ttl.kept", b"one", TTL.into()).await;
    write(bucket, "set_ttl.kept", b"two", Lifetime::Forever).await;

    wait(EXPIRED).await;

    assert_eq!(value(bucket, "set_ttl.expiring").await, None, "a set kept the old lifetime");
    let kept = value(bucket, "set_ttl.kept").await;
    assert_eq!(kept, Some(b"two".to_vec()), "a set kept the old TTL");
}

/// A zero TTL fails with the code `invalid` on every call that takes one,
/// and writes nothing.
///
/// # Panics
///
/// Panics if a zero-TTL call succeeds, fails with another code, or writes.
pub async fn zero_ttl(bucket: &Bucket) {
    write(bucket, "zero_ttl", b"old", Lifetime::Forever).await;

    let set = bucket.set("zero_ttl".to_owned(), b"new".to_vec(), Lifetime::TtlMs(0), None).await;
    let absent = Some(Precondition::Absent);
    let set_if =
        bucket.set("zero_ttl".to_owned(), b"new".to_vec(), Lifetime::KeepOrTtlMs(0), absent).await;
    let swap =
        bucket.compare_and_swap("zero_ttl", Some(b"old"), b"new", Some(Duration::ZERO)).await;
    let increment = bucket.increment("zero_ttl.counter".to_owned(), 1, Some(0)).await;

    expect_code(set, "invalid", "set with a zero TTL");
    expect_code(set_if, "invalid", "a conditional set with a zero TTL");
    expect_code(swap, "invalid", "compare_and_swap with a zero TTL");
    expect_code(increment, "invalid", "increment with a zero TTL");
    let stored = value(bucket, "zero_ttl").await;
    assert_eq!(stored, Some(b"old".to_vec()), "a zero TTL wrote the value");
    assert_eq!(counter_at(bucket, "zero_ttl.counter").await, 0, "a zero TTL wrote the counter");
}

/// A conditional [`set`](Bucket::set) under
/// [`KeepOrTtlMs`](Lifetime::KeepOrTtlMs),
/// [`compare_and_swap`](Bucket::compare_and_swap), and
/// [`increment`](Bucket::increment) apply their TTL to an entry they create.
///
/// # Panics
///
/// Panics if a created entry outlives its TTL.
pub async fn create_ttl(bucket: &Bucket, wait: impl AsyncFn(Duration)) {
    let created = conditional(bucket, "create_ttl.entry", b"new", Some(TTL), Precondition::Absent);
    created.await.expect("a create conflicted");
    let swapped = bucket.compare_and_swap("create_ttl.lock", None, b"held", Some(TTL)).await;
    assert!(swapped.expect("compare_and_swap").is_ok(), "a creating swap conflicted");
    assert_eq!(increment(bucket, "create_ttl.counter", 1, Some(TTL)).await, 1);

    wait(EXPIRED).await;

    let entry = value(bucket, "create_ttl.entry").await;
    assert_eq!(entry, None, "a created entry outlived its TTL");
    assert_eq!(value(bucket, "create_ttl.lock").await, None, "a created lock outlived its TTL");
    assert_eq!(counter_at(bucket, "create_ttl.counter").await, 0, "a counter outlived its TTL");
}

/// An update through a conditional [`set`](Bucket::set),
/// [`compare_and_swap`](Bucket::compare_and_swap), or
/// [`increment`](Bucket::increment) keeps the time the entry has left.
///
/// # Panics
///
/// Panics if an update refreshes, extends, or clears the entry's lifetime.
pub async fn update_ttl(bucket: &Bucket, wait: impl AsyncFn(Duration)) {
    write(bucket, "update_ttl.entry", b"one", TTL.into()).await;
    let stored = entry(bucket, "update_ttl.entry").await.expect("a stored entry");
    let matching = Precondition::Matches(stored.revision);
    let updated = conditional(bucket, "update_ttl.entry", b"two", Some(LONG), matching).await;
    updated.expect("a matching revision conflicted");
    write(bucket, "update_ttl.value", b"one", TTL.into()).await;
    let swapped =
        bucket.compare_and_swap("update_ttl.value", Some(b"one"), b"two", Some(LONG)).await;
    assert!(swapped.expect("compare_and_swap").is_ok(), "a matching swap conflicted");
    assert_eq!(increment(bucket, "update_ttl.counter", 1, Some(TTL)).await, 1);
    assert_eq!(increment(bucket, "update_ttl.counter", 1, Some(LONG)).await, 2);
    assert_eq!(increment(bucket, "update_ttl.counter", 1, None).await, 3);

    wait(EXPIRED).await;

    let entry = value(bucket, "update_ttl.entry").await;
    assert_eq!(entry, None, "a conditional set refreshed the lifetime");
    assert_eq!(value(bucket, "update_ttl.value").await, None, "a swap refreshed the lifetime");
    assert_eq!(counter_at(bucket, "update_ttl.counter").await, 0, "an increment refreshed it");
}

/// [`compare_and_swap`](Bucket::compare_and_swap) matches the bytes of a
/// value set with a TTL.
///
/// # Panics
///
/// Panics if the swap conflicts or does not land.
pub async fn swap_expiring(bucket: &Bucket) {
    write(bucket, "swap_expiring", b"one", LONG.into()).await;

    let swapped = bucket.compare_and_swap("swap_expiring", Some(b"one"), b"two", None).await;

    assert!(swapped.expect("compare_and_swap").is_ok(), "a TTL hid the stored bytes");
    assert_eq!(value(bucket, "swap_expiring").await, Some(b"two".to_vec()));
}

/// A [`compare_and_swap`](Bucket::compare_and_swap) that does not match
/// changes nothing and returns the entry it found.
///
/// # Panics
///
/// Panics if a mismatched swap writes or reports another value.
pub async fn conflict(bucket: &Bucket) {
    let absent = bucket.compare_and_swap("conflict", Some(b"one"), b"two", None).await;
    let absent = absent.expect("compare_and_swap").map_err(found_value);
    assert_eq!(absent, Err(None), "a swap over an absent key landed");
    assert_eq!(value(bucket, "conflict").await, None, "a conflicting swap wrote");

    write(bucket, "conflict", b"one", Lifetime::Forever).await;
    let present = bucket.compare_and_swap("conflict", None, b"two", None).await;
    let stale = bucket.compare_and_swap("conflict", Some(b"zero"), b"two", None).await;

    let found = Err(Some(b"one".to_vec()));
    assert_eq!(present.expect("compare_and_swap").map_err(found_value), found);
    assert_eq!(stale.expect("compare_and_swap").map_err(found_value), found);
    let kept = value(bucket, "conflict").await;
    assert_eq!(kept, Some(b"one".to_vec()), "a conflicting swap wrote");
}

/// A write's revision is the one [`get`](Bucket::get) reports, and a
/// conditional [`set`](Bucket::set) whose precondition does not hold changes
/// nothing and returns the entry it found.
///
/// # Panics
///
/// Panics if a revision does not match the stored state, or a failed
/// precondition writes or reports another entry.
pub async fn revision(bucket: &Bucket) {
    assert!(entry(bucket, "revision").await.is_none(), "an absent key has an entry");
    let created = conditional(bucket, "revision", b"one", None, Precondition::Absent).await;
    let created = created.expect("a create conflicted");
    let stored = entry(bucket, "revision").await.map(|stored| stored.revision);
    assert_eq!(stored.as_ref(), Some(&created), "a create wrote another revision");

    // `absent` creates once
    let again = conditional(bucket, "revision", b"two", None, Precondition::Absent).await;
    let one = Some((b"one".to_vec(), created.clone()));
    assert_eq!(again.map_err(found_entry), Err(one), "a second create landed");

    // `matches` writes over the revision it names, and conflicts on any other
    let matching = Precondition::Matches(created.clone());
    let updated = conditional(bucket, "revision", b"two", None, matching).await;
    let updated = updated.expect("a matching revision conflicted");
    assert_ne!(updated, created, "new bytes kept the revision");
    let stale = Precondition::Matches(created);
    let stale = conditional(bucket, "revision", b"three", None, stale).await;
    let two = Some((b"two".to_vec(), updated.clone()));
    assert_eq!(stale.map_err(found_entry), Err(two));

    // a write with no precondition moves the revision on
    write(bucket, "revision", b"four", Lifetime::Forever).await;
    let moved = Precondition::Matches(updated);
    let moved = conditional(bucket, "revision", b"five", None, moved).await;
    let four = Err(Some(b"four".to_vec()));
    assert_eq!(moved.map_err(found_value), four, "a set kept the revision");

    // a revision never matches an absent key
    let current = entry(bucket, "revision").await.expect("a stored entry").revision;
    let unmatched = Precondition::Matches(current);
    let absent = conditional(bucket, "revision.absent", b"v", None, unmatched).await;
    assert_eq!(absent.map_err(found_entry), Err(None));
    assert_eq!(value(bucket, "revision.absent").await, None, "an unmatched revision wrote");
}

/// [`increment`](Bucket::increment) starts an absent counter at zero, and a
/// `delta` of zero reads it.
///
/// # Panics
///
/// Panics if a counter starts elsewhere or reads back another value.
pub async fn counter(bucket: &Bucket) {
    assert_eq!(increment(bucket, "counter", 5, None).await, 5);
    assert_eq!(increment(bucket, "counter", -7, None).await, -2);
    assert_eq!(counter_at(bucket, "counter").await, -2);
}

/// Two buckets never share a key, and the rows' bucket is [`DEFAULT`].
///
/// # Panics
///
/// Panics if a write through one name reaches another.
pub async fn named(bucket: &Bucket) {
    let second = open(SECOND).await;
    write(bucket, "named", b"own", Lifetime::Forever).await;
    assert_eq!(value(&second, "named").await, None, "a second bucket saw the rows' key");

    write(&second, "named", b"second", Lifetime::Forever).await;

    let own = value(bucket, "named").await;
    assert_eq!(own, Some(b"own".to_vec()), "a second bucket overwrote the key");
    assert_eq!(value(&second, "named").await, Some(b"second".to_vec()));
    let default = value(&open(DEFAULT).await, "named").await;
    assert_eq!(default, Some(b"own".to_vec()), "the rows' bucket is not `default`");
}

/// Opening a bucket the deployment does not declare fails with the code
/// `not-found`.
///
/// # Panics
///
/// Panics if [`UNDECLARED`] opens or fails with another code.
pub async fn undeclared() {
    let opened = keyvalue::open(UNDECLARED).await;

    expect_code(opened, "not-found", "opening an undeclared bucket");
}

/// Deleting an absent key succeeds.
///
/// # Panics
///
/// Panics if a delete fails or leaves the value behind.
pub async fn delete_absent(bucket: &Bucket) {
    delete(bucket, "delete_absent").await;
    write(bucket, "delete_absent", b"v", Lifetime::Forever).await;
    delete(bucket, "delete_absent").await;
    delete(bucket, "delete_absent").await;

    assert_eq!(value(bucket, "delete_absent").await, None);
}

/// A key holding every kind of character a portable key may use works like
/// any other.
///
/// # Panics
///
/// Panics if a call on the key fails or reads back another value.
pub async fn portable_key(bucket: &Bucket) {
    write(bucket, PORTABLE, b"one", Lifetime::Forever).await;
    let swapped = bucket.compare_and_swap(PORTABLE, Some(b"one"), b"two", None).await;
    assert!(swapped.expect("compare_and_swap").is_ok(), "a matching swap conflicted");
    assert_eq!(value(bucket, PORTABLE).await, Some(b"two".to_vec()));

    delete(bucket, PORTABLE).await;

    assert_eq!(value(bucket, PORTABLE).await, None, "a delete left the portable key");
}

/// [`set_json`](Bucket::set_json) stores what [`get_json`](Bucket::get_json)
/// reads back.
///
/// # Panics
///
/// Panics if either call fails, or the value reads back changed.
pub async fn json(bucket: &Bucket) {
    let stored = serde_json::json!({"name": "ann", "tags": ["a", "b"], "rank": 3});
    bucket.set_json("json", &stored, None).await.expect("set_json");

    let read = bucket.get_json::<serde_json::Value>("json").await.expect("get_json");

    assert_eq!(read, Some(stored), "get_json read back another value");
}

/// [`get_json`](Bucket::get_json) over bytes that do not decode fails with
/// the code `invalid`.
///
/// # Panics
///
/// Panics if the read succeeds or fails with another code.
pub async fn malformed_json(bucket: &Bucket) {
    write(bucket, "malformed_json", b"{not json", Lifetime::Forever).await;

    let decoded = bucket.get_json::<serde_json::Value>("malformed_json").await;

    expect_code(decoded, "invalid", "get_json over malformed bytes");
}

async fn write(bucket: &Bucket, key: &str, value: &[u8], lifetime: Lifetime) {
    let written = bucket.set(key.to_owned(), value.to_vec(), lifetime, None).await;
    let written = written.unwrap_or_else(|err| panic!("set `{key}`: {err}"));
    written
        .unwrap_or_else(|conflict| panic!("an unconditional set `{key}` conflicted: {conflict:?}"));
}

// a TTL applies only to an entry the write creates
async fn conditional(
    bucket: &Bucket, key: &str, value: &[u8], ttl: Option<Duration>, when: Precondition,
) -> Result<Revision, Conflict> {
    let lifetime = ttl.map_or(Lifetime::Keep, |ttl| Lifetime::KeepOrTtlMs(ttl_ms(ttl)));
    let written = bucket.set(key.to_owned(), value.to_vec(), lifetime, Some(when)).await;
    written.unwrap_or_else(|err| panic!("a conditional set `{key}`: {err}"))
}

async fn delete(bucket: &Bucket, key: &str) {
    bucket.remove(key).await.unwrap_or_else(|err| panic!("delete `{key}`: {err}"));
}

async fn value(bucket: &Bucket, key: &str) -> Option<Vec<u8>> {
    entry(bucket, key).await.map(|entry| entry.value)
}

async fn entry(bucket: &Bucket, key: &str) -> Option<Entry> {
    bucket.get(key.to_owned()).await.unwrap_or_else(|err| panic!("get `{key}`: {err}"))
}

async fn increment(bucket: &Bucket, key: &str, delta: i64, ttl: Option<Duration>) -> i64 {
    let counted = bucket.increment(key.to_owned(), delta, ttl.map(ttl_ms)).await;
    counted.unwrap_or_else(|err| panic!("increment `{key}`: {err}"))
}

async fn counter_at(bucket: &Bucket, key: &str) -> i64 {
    increment(bucket, key, 0, None).await
}

async fn open(name: &str) -> Bucket {
    keyvalue::open(name).await.unwrap_or_else(|err| panic!("open `{name}`: {err}"))
}

fn found_value(conflict: Conflict) -> Option<Vec<u8>> {
    conflict.current.map(|entry| entry.value)
}

fn found_entry(conflict: Conflict) -> Option<(Vec<u8>, Revision)> {
    conflict.current.map(|entry| (entry.value, entry.revision))
}
