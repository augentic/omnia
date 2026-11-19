//! Every `omnia:docstore` error case reaches the guest as its own code: a
//! `BadGateway` for `unavailable`, and a `ServerError` for the rest. The
//! host rejects a body that is not a JSON object itself, and the default
//! backend an undeclared collection. The suite's faulting backend fails the
//! rest by collection name.

#![cfg(target_arch = "wasm32")]

use std::fmt::Debug;

use omnia_sdk::docstore::Document;
use omnia_sdk::{DocumentDb as _, Result, WasiProvider};

omnia_sdk::api::command::export!(scenario);

async fn scenario() {
    let provider = WasiProvider;

    // the host's own rule
    let raw = Document::new("raw", b"[1]");
    expect_code(provider.put("default", &raw).await, 500, "invalid");

    // the default backend's own rule
    expect_code(provider.get("undeclared", "id").await, 500, "not-found");

    // the backend's chosen cases
    for (collection, status, code) in [
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
            assert_eq!((err.status().as_u16(), err.code()), (status, code), "{err}");
        }
    }
}
