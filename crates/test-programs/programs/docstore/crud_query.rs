//! One collection walked end to end over the `wasi:docstore` guest SDK:
//! inserted bodies read back intact, a missing id reads as `None`, `put`
//! both overwrites and creates, an unfiltered query lists every document in
//! id order, a filtered query combines `and(compare, is-not-null)` with a
//! descending sort and a limit whose continuation fetches the remainder, and
//! `delete` reports whether a document was removed.

#![cfg(target_arch = "wasm32")]

use omnia_wasi_docstore::document_store::{Document, Filter, QueryOptions, SortField};
use omnia_wasi_docstore::store;
use serde_json::{Value, json};

omnia_guest::command!(scenario);

const ITEMS: &str = "items";

async fn scenario() {
    // A mix of kinds, ranks and zone presence so one filter can discriminate
    // on both a comparison and a null check; `b` is explicitly null and `e`
    // lacks the field altogether.
    insert("a", json!({"kind": "fruit", "rank": 3, "zone": "z1"})).await;
    insert("b", json!({"kind": "fruit", "rank": 1, "zone": null})).await;
    insert("c", json!({"kind": "veg", "rank": 2, "zone": "z2"})).await;
    insert("d", json!({"kind": "fruit", "rank": 2, "zone": "z3"})).await;
    insert("e", json!({"kind": "fruit", "rank": 5})).await;
    insert("g", json!({"kind": "fruit", "rank": 4, "zone": "z4"})).await;

    // Point reads: the body survives the round trip and an unknown id is
    // `None` rather than an error.
    assert_eq!(get("a").await, Some(json!({"kind": "fruit", "rank": 3, "zone": "z1"})));
    assert_eq!(get("missing").await, None);

    // Put overwrites an existing id and creates an absent one.
    put("c", json!({"kind": "veg", "rank": 9, "zone": "z2", "fresh": true})).await;
    assert_eq!(
        get("c").await,
        Some(json!({"kind": "veg", "rank": 9, "zone": "z2", "fresh": true}))
    );
    put("f", json!({"kind": "veg", "rank": 0})).await;
    assert_eq!(get("f").await, Some(json!({"kind": "veg", "rank": 0})));

    // No filter, no order: every document in id order, all on one page.
    let all = store::query(ITEMS, QueryOptions::default()).await.expect("query all");
    assert_eq!(ids(&all.documents), ["a", "b", "c", "d", "e", "f", "g"]);
    assert_eq!(all.continuation, None);

    // Fruit with a zone, highest rank first, two per page: `g`, `a`, then `d`
    // on the continuation page; `b`, `e` fail the null check, `c` and `f`
    // the comparison.
    let page = QueryOptions {
        filter: Some(Filter::and([Filter::eq("kind", "fruit"), Filter::is_not_null("zone")])),
        order_by: vec![SortField {
            field: "rank".to_owned(),
            descending: true,
        }],
        limit: Some(2),
        ..QueryOptions::default()
    };
    let first = store::query(ITEMS, page.clone()).await.expect("query first page");
    assert_eq!(ids(&first.documents), ["g", "a"]);
    let continuation = first.continuation.expect("more pages");
    let rest = store::query(
        ITEMS,
        QueryOptions {
            continuation: Some(continuation),
            ..page
        },
    )
    .await
    .expect("query last page");
    assert_eq!(ids(&rest.documents), ["d"]);
    assert_eq!(rest.continuation, None);

    // Delete reports whether anything went, and the document is gone.
    assert!(store::delete(ITEMS, "b").await.expect("delete"));
    assert!(!store::delete(ITEMS, "b").await.expect("delete absent"));
    assert_eq!(get("b").await, None);
}

async fn insert(id: &str, body: Value) {
    store::insert(ITEMS, &document(id, &body)).await.unwrap_or_else(|e| panic!("insert {id}: {e}"));
}

async fn put(id: &str, body: Value) {
    store::put(ITEMS, &document(id, &body)).await.unwrap_or_else(|e| panic!("put {id}: {e}"));
}

async fn get(id: &str) -> Option<Value> {
    let doc = store::get(ITEMS, id).await.unwrap_or_else(|e| panic!("get {id}: {e}"))?;
    assert_eq!(doc.id, id);
    Some(serde_json::from_slice(&doc.data).expect("JSON body"))
}

fn document(id: &str, body: &Value) -> Document {
    Document {
        id: id.to_owned(),
        data: serde_json::to_vec(body).expect("serialize body"),
    }
}

fn ids(documents: &[Document]) -> Vec<&str> {
    documents.iter().map(|doc| doc.id.as_str()).collect()
}
