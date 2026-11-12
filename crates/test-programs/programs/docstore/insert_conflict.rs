//! `insert` is create-only over the `omnia:docstore` guest SDK: a second
//! insert under the same id answers `Exists` and leaves the first body
//! untouched, while `put` on that id is the sanctioned overwrite.

#![cfg(target_arch = "wasm32")]

use omnia_sdk::docstore::{Document, Exists};
use omnia_sdk::{DocumentDb as _, WasiProvider};

omnia_sdk::api::command::export!(scenario);

const CONFLICT: &str = "conflict";

async fn scenario() {
    let first = WasiProvider.insert(CONFLICT, &document(br#"{"v":1}"#)).await.expect("insert");
    assert_eq!(first, Ok(()));

    let second = WasiProvider.insert(CONFLICT, &document(br#"{"v":2}"#)).await.expect("insert");
    assert_eq!(second, Err(Exists), "a duplicate id is refused");
    let kept = WasiProvider.get(CONFLICT, "dup").await.expect("get").expect("first insert stays");
    assert_eq!(&kept.data[..], br#"{"v":1}"#);

    WasiProvider.put(CONFLICT, &document(br#"{"v":3}"#)).await.expect("put overwrites");
}

fn document(data: &[u8]) -> Document {
    Document::new("dup", data)
}
