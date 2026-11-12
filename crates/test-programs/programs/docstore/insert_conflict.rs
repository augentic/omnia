//! `insert` is create-only over the `omnia:docstore` guest SDK: a second
//! insert under the same id is refused with the host's reason and leaves the
//! first body untouched, while `put` on that id is the sanctioned overwrite.

#![cfg(target_arch = "wasm32")]

use omnia_sdk::docstore::Document;
use omnia_sdk::{DocumentStore as _, WasiProvider};

omnia_sdk::api::command::export!(scenario);

const CONFLICT: &str = "conflict";

async fn scenario() {
    WasiProvider.insert(CONFLICT, &document(br#"{"v":1}"#)).await.expect("first insert");

    let error =
        WasiProvider.insert(CONFLICT, &document(br#"{"v":2}"#)).await.expect_err("duplicate id");
    assert!(error.to_string().contains("document id already exists"), "unexpected error: {error}");
    let kept = WasiProvider.get(CONFLICT, "dup").await.expect("get").expect("first insert stays");
    assert_eq!(kept.data, br#"{"v":1}"#);

    WasiProvider.put(CONFLICT, &document(br#"{"v":3}"#)).await.expect("put overwrites");
}

fn document(data: &[u8]) -> Document {
    Document {
        id: "dup".to_owned(),
        data: data.to_vec(),
    }
}
