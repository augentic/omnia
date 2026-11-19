//! One bucket walked end to end over the raw `omnia:keyvalue` bindings, on
//! the one handle `store::open` returns.
//!
//! - Increments land on the host-seeded `counter`, and `get` reads it back
//!   as decimal text.
//! - A write returns the revision a read reports. An `absent` or `matches`
//!   write that fails conflicts with the entry it found.
//! - The batch verbs answer in request order, and a key given twice is
//!   `invalid`.
//! - `keys::list` pages the keys under a prefix.
//! - A zero TTL, a non-canonical counter, and a zero `limit` are `invalid`.
//! - A `matches` write takes any lifetime. A delete with a revision removes
//!   only the entry it names, and otherwise conflicts with the entry found.

#![cfg(target_arch = "wasm32")]

use omnia_wasi_keyvalue::omnia::bucket::Bucket;
use omnia_wasi_keyvalue::omnia::types::{Conflict, Error, Lifetime, Precondition, Revision};
use omnia_wasi_keyvalue::omnia::{keys, store};

omnia_sdk::api::command::export!(scenario);

const HOUR_MS: u64 = 3_600_000;

async fn scenario() {
    let bucket = store::open("bucket".to_owned()).await.expect("open");

    // the host seeded `counter` as `37`, and an absent counter starts at zero
    let counted = bucket.increment("counter".to_owned(), 5, None).await;
    assert_eq!(counted.expect("increment"), 42);
    let counted = bucket.increment("fresh".to_owned(), 3, Some(HOUR_MS)).await;
    assert_eq!(counted.expect("increment"), 3);
    assert_eq!(value(&bucket, "counter").await.as_deref(), Some(&b"42"[..]));

    // a write returns the revision a read reports
    let first = write(&bucket, "k", b"v1", Lifetime::Forever, None).await.expect("unconditional");
    let entry = bucket.get("k".to_owned()).await.expect("get").expect("an entry");
    assert_eq!((entry.value, entry.revision), (b"v1".to_vec(), first.clone()));
    assert!(bucket.exists("k".to_owned()).await.expect("exists"));
    assert!(!bucket.exists("missing".to_owned()).await.expect("exists"));

    // `matches` writes over the revision it names, and conflicts on any other
    let matching = Some(Precondition::Matches(first.clone()));
    let second = write(&bucket, "k", b"v2", Lifetime::Keep, matching).await.expect("a match");
    assert_ne!(second, first);
    let stale = Some(Precondition::Matches(first));
    let found = write(&bucket, "k", b"v3", Lifetime::Keep, stale).await.expect_err("stale");
    assert_eq!(current(found), Some((b"v2".to_vec(), second.clone())));
    let unmatched = Some(Precondition::Matches(second));
    let found = write(&bucket, "nothing", b"v", Lifetime::Keep, unmatched).await;
    assert_eq!(current(found.expect_err("an absent key matched")), None);

    // `absent` creates an entry once
    let created = Some(Precondition::Absent);
    let lock = Lifetime::KeepOrTtlMs(HOUR_MS);
    let held = write(&bucket, "lock", b"held", lock, created.clone()).await.expect("a create");
    let found = write(&bucket, "lock", b"mine", lock, created).await.expect_err("a second create");
    assert_eq!(current(found), Some((b"held".to_vec(), held)));

    // a write that keeps the lifetime lands on an absent key and over an entry
    write(&bucket, "kept", b"one", Lifetime::Keep, None).await.expect("unconditional");
    write(&bucket, "kept", b"two", lock, None).await.expect("unconditional");
    assert_eq!(value(&bucket, "kept").await.as_deref(), Some(&b"two"[..]));

    // deleting an absent key succeeds
    bucket.delete("k".to_owned(), None).await.expect("delete").expect("unconditional");
    bucket.delete("k".to_owned(), None).await.expect("delete absent").expect("unconditional");
    assert_eq!(value(&bucket, "k").await, None);

    // the batch verbs answer in request order
    let pairs = vec![("a".to_owned(), b"1".to_vec()), ("b".to_owned(), b"2".to_vec())];
    bucket.set_many(pairs, Lifetime::Forever).await.expect("set-many");
    let many = values(&bucket, &["a", "missing", "b"]).await;
    assert_eq!(many, [Some(b"1".to_vec()), None, Some(b"2".to_vec())]);
    let deleted = vec!["a".to_owned(), "missing".to_owned()];
    bucket.delete_many(deleted).await.expect("delete-many");
    assert_eq!(values(&bucket, &["a", "b"]).await, [None, Some(b"2".to_vec())]);

    // a key given twice writes nothing
    let twice = vec![("c".to_owned(), b"1".to_vec()), ("c".to_owned(), b"2".to_vec())];
    let repeated = bucket.set_many(twice, Lifetime::Forever).await;
    assert!(matches!(repeated, Err(Error::Invalid(_))), "{repeated:?}");
    assert_eq!(value(&bucket, "c").await, None);

    // a listing pages the keys under its prefix
    for key in ["page.1", "page.2", "page.3"] {
        write(&bucket, key, b"p", Lifetime::Forever, None).await.expect("unconditional");
    }
    let page = keys::list(&bucket, "page.".to_owned(), None, Some(2)).await.expect("list");
    assert_eq!(page.keys.len(), 2, "{:?}", page.keys);
    let after = page.next.expect("a second page");
    let rest = keys::list(&bucket, "page.".to_owned(), Some(after), Some(2)).await.expect("list");
    assert_eq!(rest.next, None);
    let mut listed = [page.keys, rest.keys].concat();
    listed.sort();
    assert_eq!(listed, ["page.1", "page.2", "page.3"]);

    // refused as `invalid`
    let zero_ttl = bucket.set("zero".to_owned(), b"v".to_vec(), Lifetime::TtlMs(0), None).await;
    assert!(matches!(zero_ttl, Err(Error::Invalid(_))), "{zero_ttl:?}");
    let zero_ttl = bucket.increment("zero".to_owned(), 1, Some(0)).await;
    assert!(matches!(zero_ttl, Err(Error::Invalid(_))), "{zero_ttl:?}");
    assert_eq!(value(&bucket, "zero").await, None);
    let zero_limit = keys::list(&bucket, String::new(), None, Some(0)).await;
    assert!(matches!(zero_limit, Err(Error::Invalid(_))), "{zero_limit:?}");
    for text in ["007", "-0", "+7"] {
        write(&bucket, "padded", text.as_bytes(), Lifetime::Forever, None)
            .await
            .expect("unconditional");
        let counted = bucket.increment("padded".to_owned(), 1, None).await;
        assert!(matches!(counted, Err(Error::Invalid(_))), "`{text}` counted: {counted:?}");
    }

    // `matches` writes with a lifetime of its own
    let revision = bucket.get("b".to_owned()).await.expect("get").expect("an entry").revision;
    let matching = Some(Precondition::Matches(revision.clone()));
    let replaced = write(&bucket, "b", b"3", Lifetime::TtlMs(HOUR_MS), matching).await;
    let replaced = replaced.expect("a match");

    // a delete with a revision removes only the entry it names
    let stale = bucket.delete("b".to_owned(), Some(revision)).await.expect("delete");
    let found = current(stale.expect_err("a stale delete"));
    assert_eq!(found, Some((b"3".to_vec(), replaced.clone())));
    let deleted = bucket.delete("b".to_owned(), Some(replaced.clone())).await.expect("delete");
    deleted.expect("a matching delete");
    let absent = bucket.delete("b".to_owned(), Some(replaced)).await.expect("delete");
    assert_eq!(current(absent.expect_err("an absent key matched")), None);
}

async fn write(
    bucket: &Bucket, key: &str, value: &[u8], lifetime: Lifetime, when: Option<Precondition>,
) -> Result<Revision, Conflict> {
    let written = bucket.set(key.to_owned(), value.to_vec(), lifetime, when).await;
    written.unwrap_or_else(|err| panic!("set `{key}`: {err:?}"))
}

async fn value(bucket: &Bucket, key: &str) -> Option<Vec<u8>> {
    let entry = bucket.get(key.to_owned()).await;
    entry.unwrap_or_else(|err| panic!("get `{key}`: {err:?}")).map(|entry| entry.value)
}

async fn values(bucket: &Bucket, keys: &[&str]) -> Vec<Option<Vec<u8>>> {
    let keys = keys.iter().map(|&key| key.to_owned()).collect();
    let entries = bucket.get_many(keys).await.expect("get-many");
    entries.into_iter().map(|entry| entry.map(|entry| entry.value)).collect()
}

fn current(conflict: Conflict) -> Option<(Vec<u8>, Revision)> {
    conflict.current.map(|entry| (entry.value, entry.revision))
}
