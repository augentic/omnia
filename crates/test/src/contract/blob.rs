//! The `omnia:blobstore` rows.
//!
//! Each row uses containers of its own, so the rows run in any order, or all
//! at once, over one store. Every container and object name is portable, as
//! `omnia:blobstore` defines it.
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

use omnia_sdk::blobstore::container::{Container, Listing};
use omnia_sdk::blobstore::{admin, store};

use super::expect_code;

// a cursor still running after this many pages never ends
const MAX_PAGES: usize = 32;

// not UTF-8, so a provider that stores text cannot pass
const BINARY: &[u8] = &[0, 159, 146, 150, 255];

// the start of 2100 in milliseconds: a store that dates an object later
// dated it in a finer unit
const YEAR_2100: u64 = 4_102_444_800_000;

/// An object reads back as written, and a second
/// [`write`](Container::write) replaces all of it.
///
/// # Panics
///
/// Panics if an object reads back other bytes, keeps part of an earlier
/// write, or [`stat`](Container::stat) describes another object or dates it
/// after 2100.
pub async fn objects(scope: &str) {
    let container = create(scope, "objects").await;
    write(&container, "doc", BINARY).await;
    assert_eq!(read(&container, "doc").await, Some(BINARY.to_vec()), "a write changed bytes");

    write(&container, "doc", b"second").await;

    let stored = read(&container, "doc").await;
    assert_eq!(stored, Some(b"second".to_vec()), "a write kept part of the old object");
    let stat = container.stat("doc".to_owned()).await.expect("stat");
    let stat = stat.expect("the object is stored");
    assert_eq!((stat.name.as_str(), stat.size), ("doc", 6), "stat described another object");
    assert!(stat.modified_at < YEAR_2100, "stat dated the write after 2100");
    let exists = container.exists("doc".to_owned()).await.expect("exists");
    assert!(exists, "a stored object is absent");
    remove(&container).await;
}

/// Every lookup of a missing object in a container that exists reads it as
/// absent.
///
/// # Panics
///
/// Panics if a lookup of a missing object fails or finds one.
pub async fn absent(scope: &str) {
    let container = create(scope, "absent").await;

    let get = container.read("missing").await;
    let range = container.read_range("missing", 0..1).await;
    let empty = container.read_range("missing", 0..0).await;
    let stat = container.stat("missing".to_owned()).await;
    let exists = container.exists("missing".to_owned()).await;

    assert_eq!(get.expect("read"), None, "read found a missing object");
    assert_eq!(range.expect("read_range"), None, "read_range found a missing object");
    assert_eq!(empty.expect("read_range"), None, "an empty range found a missing object");
    assert!(stat.expect("stat").is_none(), "stat found a missing object");
    assert!(!exists.expect("exists"), "a missing object exists");
    remove(&container).await;
}

/// Deleting a missing object succeeds, and so does deleting one twice.
///
/// [`delete_many`](Container::delete_many) over a stored name and a missing
/// one deletes the stored object and leaves the rest.
///
/// # Panics
///
/// Panics if a delete fails, leaves its object behind, or deletes another.
pub async fn delete_absent(scope: &str) {
    let container = create(scope, "delete-absent").await;
    delete(&container, "missing").await;
    write(&container, "doomed", b"x").await;
    delete(&container, "doomed").await;
    delete(&container, "doomed").await;
    write(&container, "gone", b"x").await;
    write(&container, "kept", b"x").await;

    let deleted = container.delete_many(vec!["gone".to_owned(), "missing".to_owned()]).await;

    deleted.expect("delete_many with a missing name");
    assert_eq!(read(&container, "doomed").await, None, "a delete left the object");
    assert_eq!(read(&container, "gone").await, None, "delete_many left the object");
    let kept = read(&container, "kept").await;
    assert_eq!(kept, Some(b"x".to_vec()), "delete_many deleted another object");
    remove(&container).await;
}

/// A container nothing created fails with the code `not-found`, at
/// [`store::open`] or at every call on the handle it opens, and creates
/// nothing.
///
/// The container then reads as absent, and deleting it succeeds.
///
/// # Panics
///
/// Panics if a call on the container succeeds, fails with another code, or
/// creates the container.
pub async fn uncreated(scope: &str) {
    let uncreated = scoped(scope, "uncreated");
    let source = create(scope, "uncreated-source").await;
    write(&source, "doc", b"x").await;

    match store::open(uncreated.clone()).await {
        Err(err) => expect_code::<(), _>(Err(err), "not-found", "open an uncreated container"),
        Ok(handle) => {
            let get = handle.read("doc").await;
            let range = handle.read_range("doc", 0..1).await;
            let put = handle.write("doc", b"x").await;
            let delete = handle.delete("doc".to_owned(), None).await;
            let stat = handle.stat("doc".to_owned()).await;
            let list = handle.list(String::new(), Listing::Flat, None, None).await;
            let exists = handle.exists("doc".to_owned()).await;
            let delete_many = handle.delete_many(vec!["doc".to_owned()]).await;
            let clear = handle.clear().await;
            let info = handle.info().await;
            let copy = source.copy("doc".to_owned(), &handle, "doc".to_owned(), None).await;

            expect_code(get, "not-found", "read on an uncreated container");
            expect_code(range, "not-found", "read_range on an uncreated container");
            expect_code(put, "not-found", "write on an uncreated container");
            expect_code(delete, "not-found", "delete on an uncreated container");
            expect_code(stat, "not-found", "stat on an uncreated container");
            expect_code(list, "not-found", "list on an uncreated container");
            expect_code(exists, "not-found", "exists on an uncreated container");
            expect_code(delete_many, "not-found", "delete_many on an uncreated container");
            expect_code(clear, "not-found", "clear on an uncreated container");
            expect_code(info, "not-found", "info on an uncreated container");
            expect_code(copy, "not-found", "copy into an uncreated container");
        }
    }

    assert!(!container_exists(&uncreated).await, "a call created the container");
    let deleted = admin::delete_container(uncreated).await;
    deleted.expect("delete an uncreated container");
    remove(&source).await;
}

/// Creating a container twice keeps its objects, and deleting it deletes
/// them with it.
///
/// [`info`](Container::info) names a created container, and deleting a
/// container twice succeeds.
///
/// # Panics
///
/// Panics if a second create empties the container, its metadata names
/// another, or a deleted container or its objects remain.
pub async fn containers(scope: &str) {
    let container = create(scope, "containers").await;
    write(&container, "doc", b"x").await;
    let again = admin::create_container(container.name()).await;
    let again = again.expect("create the container again");

    let kept = read(&again, "doc").await;
    assert_eq!(kept, Some(b"x".to_vec()), "a second create emptied the container");
    assert!(container_exists(&container.name()).await, "a created container is absent");
    let info = container.info().await.expect("info");
    assert_eq!(info.name, container.name(), "the metadata names another container");

    admin::delete_container(container.name()).await.expect("delete the container");

    assert!(!container_exists(&container.name()).await, "a deleted container exists");
    match store::open(container.name()).await {
        Err(err) => expect_code::<(), _>(Err(err), "not-found", "open a deleted container"),
        Ok(handle) => expect_code(handle.read("doc").await, "not-found", "read a deleted one"),
    }
    let deleted = admin::delete_container(container.name()).await;
    deleted.expect("delete the container again");
}

/// [`read_range`](Container::read_range) reads a half-open range, clamped to
/// the object.
///
/// An empty range, an inverted one, and a range past the end read as empty
/// bytes.
///
/// # Panics
///
/// Panics if a range fails or reads back other bytes.
pub async fn range(scope: &str) {
    let container = create(scope, "range").await;
    write(&container, "digits", b"0123456789").await;

    #[expect(clippy::reversed_empty_ranges, reason = "an inverted range is a case")]
    let cases: [(Range<u64>, &[u8]); 7] = [
        (2..5, b"234"),
        (0..1, b"0"),
        (7..100, b"789"),
        (8..u64::MAX, b"89"),
        (4..4, b""),
        (6..3, b""),
        (20..30, b""),
    ];
    for (bounds, expected) in cases {
        let bytes = container.read_range("digits", bounds.clone()).await;
        let bytes = bytes.unwrap_or_else(|err| panic!("read_range {bounds:?}: {err}"));
        assert_eq!(bytes.as_deref(), Some(expected), "read_range {bounds:?} read other bytes");
    }
    remove(&container).await;
}

/// A [`Flat`](Listing::Flat) [`list`](Container::list) selects the objects
/// whose names start with a prefix, and its pages hold each one once.
///
/// # Panics
///
/// Panics if a listing fails or selects other objects, or its cursor repeats
/// a name, skips one, or never ends.
pub async fn listing(scope: &str) {
    let container = create(scope, "listing").await;
    for name in ["app/1", "app/2", "app/sub/3", "db/1"] {
        write(&container, name, b"x").await;
    }

    let cases: [(&str, &[&str]); 4] = [
        ("app/", &["app/1", "app/2", "app/sub/3"]),
        ("app/s", &["app/sub/3"]),
        ("", &["app/1", "app/2", "app/sub/3", "db/1"]),
        ("none/", &[]),
    ];
    for (prefix, expected) in cases {
        let names = walk(&container, prefix).await;
        assert_eq!(names, expected, "the prefix `{prefix}` selected other objects");
    }
    remove(&container).await;
}

/// [`copy`](Container::copy) duplicates an object, and
/// [`rename`](Container::rename) moves one, each replacing the destination.
///
/// A copy of a missing object fails with the code `not-found` and writes
/// nothing, and a rename onto itself keeps the object.
///
/// # Panics
///
/// Panics if a copy or a rename lands other bytes, a copy disturbs its
/// source, a rename keeps its source, or a refused copy writes.
pub async fn copy_rename(scope: &str) {
    let source = create(scope, "copy-source").await;
    let target = create(scope, "copy-target").await;
    write(&source, "doc", b"payload").await;
    write(&target, "old", b"old").await;
    let payload = Some(b"payload".to_vec());

    copy(&source, "doc", &target, "doc").await;
    copy(&source, "doc", &target, "old").await;
    assert_eq!(read(&target, "doc").await, payload, "a copy landed other bytes");
    assert_eq!(read(&target, "old").await, payload, "a copy kept the old object");
    assert_eq!(read(&source, "doc").await, payload, "a copy disturbed its source");

    let missing = source.copy("missing".to_owned(), &target, "lost".to_owned(), None).await;
    expect_code(missing, "not-found", "copy of a missing object");
    assert_eq!(read(&target, "lost").await, None, "a refused copy wrote");

    rename(&source, "doc", &source, "doc").await;
    rename(&target, "doc", &source, "moved").await;
    assert_eq!(read(&source, "doc").await, payload, "a rename onto itself lost the object");
    assert_eq!(read(&source, "moved").await, payload, "a rename landed other bytes");
    assert_eq!(read(&target, "doc").await, None, "a rename kept its source");
    remove(&source).await;
    remove(&target).await;
}

/// [`clear`](Container::clear) deletes every object and keeps the container.
///
/// # Panics
///
/// Panics if an object survives the clear, or the container does not.
pub async fn clear(scope: &str) {
    let container = create(scope, "clear").await;
    for name in ["a", "b/c"] {
        write(&container, name, b"x").await;
    }

    container.clear().await.expect("clear");

    assert!(walk(&container, "").await.is_empty(), "an object survived the clear");
    assert!(container_exists(&container.name()).await, "a clear deleted the container");
    remove(&container).await;
}

fn scoped(scope: &str, suffix: &str) -> String {
    format!("{scope}-{suffix}")
}

async fn create(scope: &str, suffix: &str) -> Container {
    let name = scoped(scope, suffix);
    let created = admin::create_container(name.clone()).await;
    created.unwrap_or_else(|err| panic!("create `{name}`: {err}"))
}

async fn write(container: &Container, name: &str, data: &[u8]) {
    let written = container.write(name, data).await;
    written.unwrap_or_else(|err| panic!("write `{name}` in `{}`: {err}", container.name()));
}

async fn read(container: &Container, name: &str) -> Option<Vec<u8>> {
    let read = container.read(name).await;
    let read = read.unwrap_or_else(|err| panic!("read `{name}` in `{}`: {err}", container.name()));
    read.map(Vec::from)
}

async fn delete(container: &Container, name: &str) {
    let deleted = container.delete(name.to_owned(), None).await;
    let deleted = deleted.unwrap_or_else(|err| panic!("delete `{name}`: {err}"));
    deleted.unwrap_or_else(|conflict| panic!("an unconditional delete conflicted: {conflict:?}"));
}

async fn copy(source: &Container, name: &str, dest: &Container, dest_name: &str) {
    let copied = source.copy(name.to_owned(), dest, dest_name.to_owned(), None).await;
    let copied = copied.unwrap_or_else(|err| panic!("copy `{name}` to `{dest_name}`: {err}"));
    copied.unwrap_or_else(|conflict| panic!("an unconditional copy conflicted: {conflict:?}"));
}

async fn rename(source: &Container, name: &str, dest: &Container, dest_name: &str) {
    let moved = source.rename(name.to_owned(), dest, dest_name.to_owned(), None).await;
    let moved = moved.unwrap_or_else(|err| panic!("rename `{name}` to `{dest_name}`: {err}"));
    moved.unwrap_or_else(|conflict| panic!("an unconditional rename conflicted: {conflict:?}"));
}

async fn container_exists(name: &str) -> bool {
    let exists = store::container_exists(name.to_owned()).await;
    exists.unwrap_or_else(|err| panic!("container_exists `{name}`: {err}"))
}

async fn remove(container: &Container) {
    let name = container.name();
    let deleted = admin::delete_container(name.clone()).await;
    deleted.unwrap_or_else(|err| panic!("delete `{name}`: {err}"));
}

// the names `prefix` selects across every page its cursor reaches, sorted
async fn walk(container: &Container, prefix: &str) -> Vec<String> {
    let mut names = Vec::new();
    let mut after = None;
    for _ in 0..MAX_PAGES {
        let page = container.list(prefix.to_owned(), Listing::Flat, after, None).await;
        let page = page.unwrap_or_else(|err| panic!("list `{prefix}`: {err}"));
        names.extend(page.objects.into_iter().map(|object| object.name));
        after = page.next;
        if after.is_none() {
            names.sort();
            return names;
        }
    }
    panic!("the cursor did not end within {MAX_PAGES} pages");
}
