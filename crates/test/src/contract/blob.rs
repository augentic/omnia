//! The [`BlobStore`] rows.
//!
//! Each row uses containers of its own, so the rows run in any order, or all
//! at once, over one store. Every container and object name is portable, as
//! [`BlobStore`] defines it.
//!
//! A row names each container `<scope>-<suffix>`, with suffixes of its own:
//!
//! - the store must start without any of them;
//! - a passing row deletes every container it created;
//! - a `scope` is a portable container name of at most 40 characters.
//!
//! A store that holds a deleted container's name for a while, as Azure Blob
//! Storage does, runs the rows again under a fresh `scope`.

use std::ops::Range;

use omnia_sdk::BlobStore;
use omnia_sdk::capability::ErrorKind;

use super::expect_kind;

// a cursor still running after this many pages never ends
const MAX_PAGES: usize = 32;

// not UTF-8, so a provider that stores text cannot pass
const BINARY: &[u8] = &[0, 159, 146, 150, 255];

/// An object reads back as written, and a second [`put`](BlobStore::put)
/// replaces all of it.
///
/// # Panics
///
/// Panics if an object reads back other bytes, keeps part of an earlier
/// write, or [`stat`](BlobStore::stat) describes another object.
pub async fn objects(store: &impl BlobStore, scope: &str) {
    let container = create(store, scope, "objects").await;
    write(store, &container, "doc", BINARY).await;
    assert_eq!(read(store, &container, "doc").await, Some(BINARY.to_vec()), "a put changed bytes");

    write(store, &container, "doc", b"second").await;

    let stored = read(store, &container, "doc").await;
    assert_eq!(stored, Some(b"second".to_vec()), "a put kept part of the old object");
    let stat = store.stat(&container, "doc").await.expect("stat").expect("the object is stored");
    assert_eq!((stat.name.as_str(), stat.container.as_str(), stat.size), ("doc", &*container, 6));
    assert!(store.exists(&container, "doc").await.expect("exists"), "a stored object is absent");
    remove(store, &container).await;
}

/// Every lookup of a missing object in a container that exists reads it as
/// absent.
///
/// # Panics
///
/// Panics if a lookup of a missing object fails or finds one.
pub async fn absent(store: &impl BlobStore, scope: &str) {
    let container = create(store, scope, "absent").await;

    let get = store.get(&container, "missing").await;
    let range = store.get_range(&container, "missing", 0..1).await;
    let empty = store.get_range(&container, "missing", 0..0).await;
    let stat = store.stat(&container, "missing").await;
    let exists = store.exists(&container, "missing").await;

    assert_eq!(get.expect("get"), None, "get found a missing object");
    assert_eq!(range.expect("get_range"), None, "get_range found a missing object");
    assert_eq!(empty.expect("get_range"), None, "an empty range found a missing object");
    assert_eq!(stat.expect("stat"), None, "stat found a missing object");
    assert!(!exists.expect("exists"), "a missing object exists");
    remove(store, &container).await;
}

/// Deleting a missing object succeeds, and so does deleting one twice.
///
/// [`delete_many`](BlobStore::delete_many) over a stored name and a missing
/// one deletes the stored object and leaves the rest.
///
/// # Panics
///
/// Panics if a delete fails, leaves its object behind, or deletes another.
pub async fn delete_absent(store: &impl BlobStore, scope: &str) {
    let container = create(store, scope, "delete-absent").await;
    store.delete(&container, "missing").await.expect("delete a missing object");
    write(store, &container, "doomed", b"x").await;
    store.delete(&container, "doomed").await.expect("delete");
    store.delete(&container, "doomed").await.expect("delete the object again");
    write(store, &container, "gone", b"x").await;
    write(store, &container, "kept", b"x").await;

    let deleted = store.delete_many(&container, &["gone", "missing"]).await;

    deleted.expect("delete_many with a missing name");
    assert_eq!(read(store, &container, "doomed").await, None, "a delete left the object");
    assert_eq!(read(store, &container, "gone").await, None, "delete_many left the object");
    let kept = read(store, &container, "kept").await;
    assert_eq!(kept, Some(b"x".to_vec()), "delete_many deleted another object");
    remove(store, &container).await;
}

/// Every object call on a container nothing created fails with
/// [`ErrorKind::NotFound`] and creates nothing.
///
/// The container then reads as absent, and deleting it succeeds.
///
/// # Panics
///
/// Panics if an object call on the container succeeds, fails with another
/// kind, or creates the container.
pub async fn uncreated(store: &impl BlobStore, scope: &str) {
    let uncreated = scoped(scope, "uncreated");
    let source = create(store, scope, "uncreated-source").await;
    write(store, &source, "doc", b"x").await;

    let get = store.get(&uncreated, "doc").await;
    let range = store.get_range(&uncreated, "doc", 0..1).await;
    let put = store.put(&uncreated, "doc", b"x").await;
    let delete = store.delete(&uncreated, "doc").await;
    let stat = store.stat(&uncreated, "doc").await;
    let list = store.list(&uncreated, "", None).await;
    let exists = store.exists(&uncreated, "doc").await;
    let delete_many = store.delete_many(&uncreated, &["doc"]).await;
    let clear = store.clear(&uncreated).await;
    let copy = store.copy(&source, "doc", &uncreated, "doc").await;

    expect_kind(get, ErrorKind::NotFound, "get on an uncreated container");
    expect_kind(range, ErrorKind::NotFound, "get_range on an uncreated container");
    expect_kind(put, ErrorKind::NotFound, "put on an uncreated container");
    expect_kind(delete, ErrorKind::NotFound, "delete on an uncreated container");
    expect_kind(stat, ErrorKind::NotFound, "stat on an uncreated container");
    expect_kind(list, ErrorKind::NotFound, "list on an uncreated container");
    expect_kind(exists, ErrorKind::NotFound, "exists on an uncreated container");
    expect_kind(delete_many, ErrorKind::NotFound, "delete_many on an uncreated container");
    expect_kind(clear, ErrorKind::NotFound, "clear on an uncreated container");
    expect_kind(copy, ErrorKind::NotFound, "copy into an uncreated container");
    let exists = store.container_exists(&uncreated).await.expect("container_exists");
    assert!(!exists, "an object call created the container");
    let info = store.container_info(&uncreated).await.expect("container_info");
    assert_eq!(info, None, "an uncreated container has metadata");
    store.delete_container(&uncreated).await.expect("delete an uncreated container");
    remove(store, &source).await;
}

/// Creating a container twice keeps its objects, and deleting it deletes
/// them with it.
///
/// [`container_info`](BlobStore::container_info) names a created container,
/// and deleting a container twice succeeds.
///
/// # Panics
///
/// Panics if a second create empties the container, its metadata names
/// another, or a deleted container or its objects remain.
pub async fn containers(store: &impl BlobStore, scope: &str) {
    let container = create(store, scope, "containers").await;
    write(store, &container, "doc", b"x").await;
    store.create_container(&container).await.expect("create the container again");

    let kept = read(store, &container, "doc").await;
    assert_eq!(kept, Some(b"x".to_vec()), "a second create emptied the container");
    let exists = store.container_exists(&container).await.expect("container_exists");
    assert!(exists, "a created container is absent");
    let info = store.container_info(&container).await.expect("container_info");
    let info = info.expect("a created container has metadata");
    assert_eq!(info.name, container, "the metadata names another container");

    store.delete_container(&container).await.expect("delete the container");

    let exists = store.container_exists(&container).await.expect("container_exists");
    assert!(!exists, "a deleted container exists");
    let get = store.get(&container, "doc").await;
    expect_kind(get, ErrorKind::NotFound, "get in a deleted container");
    store.delete_container(&container).await.expect("delete the container again");
}

/// [`get_range`](BlobStore::get_range) reads a half-open range, clamped to
/// the object.
///
/// An empty range, and a range past the end, read as empty bytes.
///
/// # Panics
///
/// Panics if a range fails or reads back other bytes.
pub async fn range(store: &impl BlobStore, scope: &str) {
    let container = create(store, scope, "range").await;
    write(store, &container, "digits", b"0123456789").await;

    let cases: [(Range<u64>, &[u8]); 5] =
        [(2..5, b"234"), (0..1, b"0"), (7..100, b"789"), (4..4, b""), (20..30, b"")];
    for (bounds, expected) in cases {
        let bytes = store.get_range(&container, "digits", bounds.clone()).await;
        let bytes = bytes.unwrap_or_else(|err| panic!("get_range {bounds:?}: {err}"));
        assert_eq!(bytes.as_deref(), Some(expected), "get_range {bounds:?} read other bytes");
    }
    remove(store, &container).await;
}

/// [`list`](BlobStore::list) selects the objects whose names start with a
/// prefix, and its pages hold each one once.
///
/// # Panics
///
/// Panics if a listing fails or selects other objects, or its cursor repeats
/// a name, skips one, or never ends.
pub async fn listing(store: &impl BlobStore, scope: &str) {
    let container = create(store, scope, "listing").await;
    for name in ["app/1", "app/2", "app/sub/3", "db/1"] {
        write(store, &container, name, b"x").await;
    }

    let cases: [(&str, &[&str]); 4] = [
        ("app/", &["app/1", "app/2", "app/sub/3"]),
        ("app/s", &["app/sub/3"]),
        ("", &["app/1", "app/2", "app/sub/3", "db/1"]),
        ("none/", &[]),
    ];
    for (prefix, expected) in cases {
        let names = walk(store, &container, prefix).await;
        assert_eq!(names, expected, "the prefix `{prefix}` selected other objects");
    }
    remove(store, &container).await;
}

/// [`copy`](BlobStore::copy) duplicates an object, and
/// [`rename`](BlobStore::rename) moves one, each replacing the destination.
///
/// A copy of a missing object fails with [`ErrorKind::NotFound`] and writes
/// nothing, and a rename onto itself keeps the object.
///
/// # Panics
///
/// Panics if a copy or a rename lands other bytes, a copy disturbs its
/// source, a rename keeps its source, or a refused copy writes.
pub async fn copy_rename(store: &impl BlobStore, scope: &str) {
    let source = create(store, scope, "copy-source").await;
    let target = create(store, scope, "copy-target").await;
    write(store, &source, "doc", b"payload").await;
    write(store, &target, "old", b"old").await;
    let payload = Some(b"payload".to_vec());

    store.copy(&source, "doc", &target, "doc").await.expect("copy to a new name");
    store.copy(&source, "doc", &target, "old").await.expect("copy over an object");
    assert_eq!(read(store, &target, "doc").await, payload, "a copy landed other bytes");
    assert_eq!(read(store, &target, "old").await, payload, "a copy kept the old object");
    assert_eq!(read(store, &source, "doc").await, payload, "a copy disturbed its source");

    let missing = store.copy(&source, "missing", &target, "lost").await;
    expect_kind(missing, ErrorKind::NotFound, "copy of a missing object");
    assert_eq!(read(store, &target, "lost").await, None, "a refused copy wrote");

    store.rename(&source, "doc", &source, "doc").await.expect("rename onto itself");
    store.rename(&target, "doc", &source, "moved").await.expect("rename across containers");
    assert_eq!(read(store, &source, "doc").await, payload, "a rename onto itself lost the object");
    assert_eq!(read(store, &source, "moved").await, payload, "a rename landed other bytes");
    assert_eq!(read(store, &target, "doc").await, None, "a rename kept its source");
    remove(store, &source).await;
    remove(store, &target).await;
}

/// [`clear`](BlobStore::clear) deletes every object and keeps the container.
///
/// # Panics
///
/// Panics if an object survives the clear, or the container does not.
pub async fn clear(store: &impl BlobStore, scope: &str) {
    let container = create(store, scope, "clear").await;
    for name in ["a", "b/c"] {
        write(store, &container, name, b"x").await;
    }

    store.clear(&container).await.expect("clear");

    assert!(walk(store, &container, "").await.is_empty(), "an object survived the clear");
    let exists = store.container_exists(&container).await.expect("container_exists");
    assert!(exists, "a clear deleted the container");
    remove(store, &container).await;
}

fn scoped(scope: &str, suffix: &str) -> String {
    format!("{scope}-{suffix}")
}

async fn create(store: &impl BlobStore, scope: &str, suffix: &str) -> String {
    let container = scoped(scope, suffix);
    let created = store.create_container(&container).await;
    created.unwrap_or_else(|err| panic!("create `{container}`: {err}"));
    container
}

async fn write(store: &impl BlobStore, container: &str, name: &str, data: &[u8]) {
    let written = store.put(container, name, data).await;
    written.unwrap_or_else(|err| panic!("put `{name}` in `{container}`: {err}"));
}

async fn read(store: &impl BlobStore, container: &str, name: &str) -> Option<Vec<u8>> {
    let read = store.get(container, name).await;
    read.unwrap_or_else(|err| panic!("get `{name}` in `{container}`: {err}")).map(Vec::from)
}

async fn remove(store: &impl BlobStore, container: &str) {
    let deleted = store.delete_container(container).await;
    deleted.unwrap_or_else(|err| panic!("delete `{container}`: {err}"));
}

// the names `prefix` selects across every page its cursor reaches, sorted
async fn walk(store: &impl BlobStore, container: &str, prefix: &str) -> Vec<String> {
    let mut names = Vec::new();
    let mut after = None;
    for _ in 0..MAX_PAGES {
        let page = store.list(container, prefix, after.as_deref()).await;
        let page = page.unwrap_or_else(|err| panic!("list `{prefix}` in `{container}`: {err}"));
        names.extend(page.items.into_iter().map(|object| object.name));
        after = page.next;
        if after.is_none() {
            names.sort();
            return names;
        }
    }
    panic!("the cursor did not end within {MAX_PAGES} pages");
}
