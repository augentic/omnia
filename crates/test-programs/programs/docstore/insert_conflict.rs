//! `insert` is create-only over the `wasi:docstore` guest SDK: a second
//! insert under the same id is refused with the host's reason and leaves the
//! first body untouched, while `put` on that id is the sanctioned overwrite.

#![cfg(target_arch = "wasm32")]

use omnia_wasi_docstore::document_store::Document;
use omnia_wasi_docstore::store;

omnia_guest::command!(scenario);

const CONFLICT: &str = "conflict";

async fn scenario() {
    store::insert(CONFLICT, &document(br#"{"v":1}"#)).await.expect("first insert");

    let error = store::insert(CONFLICT, &document(br#"{"v":2}"#)).await.expect_err("duplicate id");
    assert!(error.to_string().contains("document id already exists"), "unexpected error: {error}");
    let kept = store::get(CONFLICT, "dup").await.expect("get").expect("first insert stays");
    assert_eq!(kept.data, br#"{"v":1}"#);

    store::put(CONFLICT, &document(br#"{"v":3}"#)).await.expect("put overwrites");
}

fn document(data: &[u8]) -> Document {
    Document {
        id: "dup".to_owned(),
        data: data.to_vec(),
    }
}
