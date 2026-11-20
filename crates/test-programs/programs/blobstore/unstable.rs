//! Raw bindings: the unstable `uploads` and `presign` interfaces link, and
//! the in-memory default answers each call with `unsupported`.

#![cfg(target_arch = "wasm32")]

use omnia_sdk::blobstore::types::{Attributes, Error};
use omnia_sdk::blobstore::{presign, store, uploads};

omnia_sdk::command::export!(scenario);

async fn scenario() {
    let objects = store::open("objects".to_owned()).await.expect("open the seeded container");

    let begun = uploads::begin(&objects, "big".to_owned(), Attributes::default()).await;
    unsupported(begun.err(), "no multipart uploads");
    let resumed = uploads::resume(&objects, "upload-1".to_owned()).await;
    unsupported(resumed.err(), "no multipart uploads");

    let read = presign::read(&objects, "big".to_owned(), 60_000).await;
    unsupported(read.err(), "no presigned requests");
    let written =
        presign::write(&objects, "big".to_owned(), Attributes::default(), None, 60_000).await;
    unsupported(written.err(), "no presigned requests");
}

#[track_caller]
fn unsupported(error: Option<Error>, needle: &str) {
    assert!(
        matches!(&error, Some(Error::Unsupported(detail)) if detail.contains(needle)),
        "expected `unsupported` holding {needle:?}, got {error:?}"
    );
}
