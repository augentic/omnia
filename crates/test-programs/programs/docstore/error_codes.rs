//! Every `omnia:docstore` error case reaches the guest as its own code: a
//! `BadGateway` for `unavailable`, and a `ServerError` for the rest. The
//! host rejects a body that is not a JSON object itself, and the default
//! backend an undeclared collection. The suite's faulting backend fails the
//! rest by collection name.

#![cfg(target_arch = "wasm32")]

use std::fmt::Debug;

use omnia_sdk::docstore::store;
use omnia_sdk::docstore::types::Document;
use omnia_sdk::{Error, Result};

omnia_sdk::command::export!(scenario);

async fn scenario() {
    // the host's own rule
    let default = store::open("default".to_owned()).await.expect("open the default collection");
    let raw = default.put("raw".to_owned(), "[1]".to_owned(), None).await;
    expect_code(raw.map_err(Error::from), 500, "invalid");

    // the default backend's own rule
    expect_code(get("undeclared", "id").await, 500, "not-found");

    // the backend's chosen cases
    for (collection, status, code) in [
        ("denied", 500, "denied"),
        ("unsupported", 500, "unsupported"),
        ("unavailable", 502, "unavailable"),
        ("other", 500, "other"),
    ] {
        expect_code(get(collection, "id").await, status, code);
    }
}

async fn get(collection: &str, id: &str) -> Result<Option<Document>> {
    let collection = store::open(collection.to_owned()).await?;
    Ok(collection.get(id.to_owned()).await?)
}

fn expect_code<T: Debug>(result: Result<T>, status: u16, code: &str) {
    match result {
        Ok(value) => panic!("returned {value:?}, not a `{code}` failure"),
        Err(err) => {
            assert_eq!((err.status().as_u16(), err.code()), (status, code), "{err}");
        }
    }
}
