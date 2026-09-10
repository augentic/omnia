//! Cross-container transfers over the raw `wasi:blobstore` bindings:
//! `copy-object` duplicates into another container and overwrites an existing
//! destination while the source stays put, refuses a missing source object or
//! destination container, `move-object` onto itself is a no-op, and a real
//! move leaves the destination holding what the source gave up.

#![cfg(target_arch = "wasm32")]

use omnia_wasi_blobstore::blobstore;
use omnia_wasi_blobstore::container::Container;
use omnia_wasi_blobstore::types::{IncomingValue, ObjectId, OutgoingValue};

omnia_guest::command!(scenario);

async fn scenario() {
    let a = blobstore::create_container("a".to_owned()).await.expect("create a");
    let b = blobstore::create_container("b".to_owned()).await.expect("create b");
    let c = blobstore::create_container("c".to_owned()).await.expect("create c");
    write(&a, "doc", b"payload").await;

    // Copy duplicates; the source is untouched.
    blobstore::copy_object(id("a", "doc"), id("b", "doc")).await.expect("copy a/doc -> b/doc");
    assert_eq!(read(&a, "doc").await, b"payload");
    assert_eq!(read(&b, "doc").await, b"payload");

    // Copy overwrites an existing destination.
    write(&b, "doc2", b"old").await;
    blobstore::copy_object(id("a", "doc"), id("b", "doc2")).await.expect("copy a/doc -> b/doc2");
    assert_eq!(read(&b, "doc2").await, b"payload");

    // A missing source object or destination container is an error, and
    // nothing lands.
    let error =
        blobstore::copy_object(id("a", "missing"), id("b", "x")).await.expect_err("missing source");
    assert!(error.contains("source object not found"), "unexpected error: {error}");
    assert!(!b.has_object("x".to_owned()).await.expect("has-object"));
    let error = blobstore::copy_object(id("a", "doc"), id("nowhere", "doc"))
        .await
        .expect_err("missing destination container");
    assert!(error.contains("container not found"), "unexpected error: {error}");
    assert!(!blobstore::container_exists("nowhere".to_owned()).await.expect("exists"));

    // Moving onto itself must not delete the object.
    blobstore::move_object(id("a", "doc"), id("a", "doc")).await.expect("move onto itself");
    assert_eq!(read(&a, "doc").await, b"payload");

    // A real move relocates: the source is gone, the destination holds it,
    // and the unrelated original is untouched.
    blobstore::move_object(id("b", "doc"), id("c", "doc")).await.expect("move b/doc -> c/doc");
    assert!(!b.has_object("doc".to_owned()).await.expect("has-object"));
    assert_eq!(read(&c, "doc").await, b"payload");
    assert_eq!(read(&a, "doc").await, b"payload");
}

fn id(container: &str, object: &str) -> ObjectId {
    ObjectId {
        container: container.to_owned(),
        object: object.to_owned(),
    }
}

async fn write(container: &Container, name: &str, data: &[u8]) {
    let outgoing = OutgoingValue::new_outgoing_value();
    {
        let body = outgoing.outgoing_value_write_body().await.expect("write-body");
        body.blocking_write_and_flush(data).expect("write");
    }
    container.write_data(name.to_owned(), &outgoing).await.expect("write-data");
    OutgoingValue::finish(outgoing).expect("finish");
}

async fn read(container: &Container, name: &str) -> Vec<u8> {
    let incoming = container.get_data(name.to_owned(), 0, u64::MAX).await.expect("get-data");
    IncomingValue::incoming_value_consume_sync(incoming).expect("consume")
}
