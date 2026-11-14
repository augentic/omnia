//! Every `omnia:docstore` error case reaches the guest as its own code: a
//! `BadGateway` for `unavailable`, and a `ServerError` for the rest. The
//! default backend fails a body that is not JSON itself. The suite's
//! faulting backend fails the rest by collection name.

#![cfg(target_arch = "wasm32")]

use std::fmt::Debug;

use omnia_sdk::docstore::Document;
use omnia_sdk::{DocumentDb as _, Result, WasiProvider};

omnia_sdk::api::command::export!(scenario);

async fn scenario() {
    let provider = WasiProvider;

    // the default backend's own case
    let raw = Document::new("raw", b"not json");
    expect_code(provider.put("bodies", &raw).await, 500, "invalid");

    // the backend's chosen cases
    for (collection, status, code) in [
        ("missing", 500, "not-found"),
        ("denied", 500, "denied"),
        ("unsupported", 500, "unsupported"),
        ("unavailable", 502, "unavailable"),
        ("other", 500, "other"),
    ] {
        expect_code(provider.get(collection, "id").await, status, code);
    }
}

fn expect_code<T: Debug>(result: Result<T>, status: u16, code: &str) {
    match result {
        Ok(value) => panic!("returned {value:?}, not a `{code}` failure"),
        Err(err) => {
            assert_eq!((err.status().as_u16(), err.code().as_str()), (status, code), "{err}");
        }
    }
}
