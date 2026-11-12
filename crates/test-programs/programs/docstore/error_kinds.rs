//! Every `omnia:docstore` error kind reaches the guest as its own
//! `ErrorKind`. The default backend fails a body that is not JSON itself.
//! The suite's faulting backend fails the rest by collection name.

#![cfg(target_arch = "wasm32")]

use std::fmt::Debug;

use omnia_sdk::capability::{ErrorKind, Result};
use omnia_sdk::docstore::Document;
use omnia_sdk::{DocumentDb as _, WasiProvider};

omnia_sdk::api::command::export!(scenario);

async fn scenario() {
    let provider = WasiProvider;

    // the default backend's own kind
    let raw = Document::new("raw", b"not json");
    expect_kind(provider.put("bodies", &raw).await, ErrorKind::Invalid);

    // the backend's chosen kinds
    for (collection, kind) in [
        ("missing", ErrorKind::NotFound),
        ("denied", ErrorKind::Denied),
        ("unsupported", ErrorKind::Unsupported),
        ("unavailable", ErrorKind::Unavailable),
        ("other", ErrorKind::Other),
    ] {
        expect_kind(provider.get(collection, "id").await, kind);
    }
}

fn expect_kind<T: Debug>(result: Result<T>, kind: ErrorKind) {
    match result {
        Ok(value) => panic!("returned {value:?}, not a `{kind}` failure"),
        Err(err) => assert_eq!(err.kind(), kind, "{err}"),
    }
}
