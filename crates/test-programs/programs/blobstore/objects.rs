//! One container walked end to end over the raw `wasi:blobstore` bindings: a
//! host-seeded container is readable, a chunked `outgoing-value` lands as one
//! object, `get-data` honours whole and inclusive ranged reads, `has-object`
//! and `object-info` distinguish present from absent, `list-objects` pages
//! and skips, single and batch deletes are idempotent, `clear` empties, and
//! `delete-container` removes the container.

#![cfg(target_arch = "wasm32")]

use omnia_wasi_blobstore::blobstore;
use omnia_wasi_blobstore::container::Container;
use omnia_wasi_blobstore::types::{IncomingValue, OutgoingValue};

omnia_guest::command!(scenario);

/// Longer than one `blocking-write-and-flush` budget so the write spans
/// several stream chunks the host must accumulate into one value.
const LARGE: usize = 10_000;

async fn scenario() {
    // The host seeded `seeded/from-host` before the run; a missing container
    // is an error rather than an empty one.
    let seeded = blobstore::get_container("seeded".to_owned()).await.expect("get-container");
    assert_eq!(read(&seeded, "from-host").await, b"hello");
    let error = blobstore::get_container("missing".to_owned()).await.expect_err("absent");
    assert!(error.contains("container not found"), "unexpected error: {error}");

    // Create is observable and idempotent.
    assert!(!blobstore::container_exists("objects".to_owned()).await.expect("exists"));
    let container = blobstore::create_container("objects".to_owned()).await.expect("create");
    assert!(blobstore::container_exists("objects".to_owned()).await.expect("exists"));
    assert_eq!(container.name().expect("name"), "objects");
    assert_eq!(container.info().expect("info").name, "objects");

    let large: Vec<u8> = (0..LARGE).map(|i| (i % 251) as u8).collect();
    write(&container, "alpha", &large).await;
    let again = blobstore::create_container("objects".to_owned()).await.expect("re-create");
    assert!(again.has_object("alpha".to_owned()).await.expect("has-object"), "create kept data");

    // Read back whole, then an inclusive range; a missing object is an error.
    let incoming = container.get_data("alpha".to_owned(), 0, u64::MAX).await.expect("get-data");
    assert_eq!(incoming.size(), LARGE as u64);
    assert_eq!(IncomingValue::incoming_value_consume_sync(incoming).expect("consume"), large);
    let incoming = container.get_data("alpha".to_owned(), 2, 5).await.expect("ranged get-data");
    assert_eq!(incoming.size(), 4);
    assert_eq!(
        IncomingValue::incoming_value_consume_sync(incoming).expect("consume"),
        large[2..=5]
    );
    let error = container.get_data("missing".to_owned(), 0, u64::MAX).await.expect_err("absent");
    assert!(error.contains("object not found"), "unexpected error: {error}");

    // Presence and metadata.
    assert!(container.has_object("alpha".to_owned()).await.expect("has-object"));
    assert!(!container.has_object("missing".to_owned()).await.expect("has-object"));
    let info = container.object_info("alpha".to_owned()).await.expect("object-info");
    assert_eq!(info.name, "alpha");
    assert_eq!(info.container, "objects");
    assert_eq!(info.size, LARGE as u64);
    let error = container.object_info("missing".to_owned()).await.expect_err("absent");
    assert!(error.contains("object not found"), "unexpected error: {error}");

    // Overwrite replaces the whole value.
    write(&container, "alpha", b"short").await;
    assert_eq!(read(&container, "alpha").await, b"short");

    // Listing pages through every name and reports the end once.
    write(&container, "beta", b"b").await;
    write(&container, "gamma", b"g").await;
    let stream = container.list_objects().await.expect("list-objects");
    let (first, done) = stream.read_stream_object_names(1).await.expect("read one");
    assert_eq!(first.len(), 1);
    assert!(!done);
    let (rest, done) = stream.read_stream_object_names(10).await.expect("read rest");
    assert!(done);
    let mut names = first;
    names.extend(rest);
    names.sort();
    assert_eq!(names, ["alpha", "beta", "gamma"]);

    let stream = container.list_objects().await.expect("list-objects");
    assert_eq!(stream.skip_stream_object_names(2).await.expect("skip"), (2, false));
    let (last, done) = stream.read_stream_object_names(10).await.expect("read last");
    assert_eq!(last.len(), 1);
    assert!(done);

    // Single delete is idempotent; batch delete tolerates an absent name.
    container.delete_object("beta".to_owned()).await.expect("delete-object");
    assert!(!container.has_object("beta".to_owned()).await.expect("has-object"));
    container.delete_object("beta".to_owned()).await.expect("delete absent");
    container
        .delete_objects(vec!["gamma".to_owned(), "missing".to_owned()])
        .await
        .expect("delete-objects");
    assert!(!container.has_object("gamma".to_owned()).await.expect("has-object"));
    assert!(container.has_object("alpha".to_owned()).await.expect("has-object"));

    // Clear empties the container but keeps it.
    container.clear().await.expect("clear");
    let stream = container.list_objects().await.expect("list-objects");
    assert_eq!(stream.read_stream_object_names(10).await.expect("read"), (vec![], true));
    assert!(blobstore::container_exists("objects".to_owned()).await.expect("exists"));

    // Delete removes the container and everything written since the clear.
    write(&container, "doomed", b"x").await;
    blobstore::delete_container("objects".to_owned()).await.expect("delete-container");
    assert!(!blobstore::container_exists("objects".to_owned()).await.expect("exists"));

    // The host reads this back after the run.
    write(&seeded, "reply", b"from-guest").await;
}

async fn write(container: &Container, name: &str, data: &[u8]) {
    let outgoing = OutgoingValue::new_outgoing_value();
    {
        let body = outgoing.outgoing_value_write_body().await.expect("write-body");
        // `blocking-write-and-flush` accepts at most 4096 bytes per call.
        for chunk in data.chunks(4096) {
            body.blocking_write_and_flush(chunk).expect("write chunk");
        }
    }
    container.write_data(name.to_owned(), &outgoing).await.expect("write-data");
    OutgoingValue::finish(outgoing).expect("finish");
}

async fn read(container: &Container, name: &str) -> Vec<u8> {
    let incoming = container.get_data(name.to_owned(), 0, u64::MAX).await.expect("get-data");
    IncomingValue::incoming_value_consume_sync(incoming).expect("consume")
}
