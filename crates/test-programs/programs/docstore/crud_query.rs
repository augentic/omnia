//! End-to-end walk over the `omnia:docstore` guest SDK.
//!
//! - Inserted bodies read back intact, and an unknown id reads as `None`.
//! - `put` overwrites an existing id and creates an absent one.
//! - Queries list in id order, filter, sort with type rank, and paginate.
//! - `delete` reports whether a document was removed.

#![cfg(target_arch = "wasm32")]

use omnia_sdk::docstore::{Document, Filter, QueryOptions, SortField};
use omnia_sdk::{DocumentStore as _, WasiProvider};
use serde_json::{Value, json};

omnia_sdk::api::command::export!(scenario);

const ITEMS: &str = "items";
const EDGES: &str = "edges";

async fn scenario() {
    // seed documents mixing present, explicit null, and absent fields
    insert("a", json!({"kind": "fruit", "rank": 3, "zone": "z1"})).await;
    insert("b", json!({"kind": "fruit", "rank": 1, "zone": null})).await;
    insert("c", json!({"kind": "veg", "rank": 2, "zone": "z2"})).await;
    insert("d", json!({"kind": "fruit", "rank": 2, "zone": "z3"})).await;
    insert("e", json!({"kind": "fruit", "rank": 5})).await;
    insert("g", json!({"kind": "fruit", "rank": 4, "zone": "z4"})).await;

    // point reads return the stored body or none
    assert_eq!(get("a").await, Some(json!({"kind": "fruit", "rank": 3, "zone": "z1"})));
    assert_eq!(get("missing").await, None);

    // put overwrites an existing id and creates an absent one
    put("c", json!({"kind": "veg", "rank": 9, "zone": "z2", "fresh": true})).await;
    assert_eq!(
        get("c").await,
        Some(json!({"kind": "veg", "rank": 9, "zone": "z2", "fresh": true}))
    );
    put("f", json!({"kind": "veg", "rank": 0})).await;
    assert_eq!(get("f").await, Some(json!({"kind": "veg", "rank": 0})));

    // unfiltered query lists every document in id order on one page
    let all = WasiProvider.query(ITEMS, QueryOptions::default()).await.expect("query all");
    assert_eq!(ids(&all.documents), ["a", "b", "c", "d", "e", "f", "g"]);
    assert_eq!(all.continuation, None);

    // fruit with a zone, highest rank first, two per page
    let page = QueryOptions {
        filter: Some(Filter::and([Filter::eq("kind", "fruit"), Filter::is_not_null("zone")])),
        order_by: vec![SortField {
            field: "rank".to_owned(),
            descending: true,
        }],
        limit: Some(2),
        ..QueryOptions::default()
    };
    let first = WasiProvider.query(ITEMS, page.clone()).await.expect("query first page");
    assert_eq!(ids(&first.documents), ["g", "a"]);
    let continuation = first.continuation.expect("more pages");
    let rest = WasiProvider
        .query(
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

    // delete reports whether a document was removed
    assert!(WasiProvider.delete(ITEMS, "b").await.expect("delete"));
    assert!(!WasiProvider.delete(ITEMS, "b").await.expect("delete absent"));
    assert_eq!(get("b").await, None);

    // absent or incomparable fields across operators and ascending sort order
    for (id, body) in [
        ("a", json!({"v": 5})),
        ("m", json!({"v": 2.0})),
        ("n", json!({"v": 15})),
        ("s", json!({"v": "1.5"})),
        ("z", json!({})),
    ] {
        WasiProvider
            .insert(EDGES, &document(id, &body))
            .await
            .unwrap_or_else(|e| panic!("insert {id}: {e}"));
    }
    assert_eq!(filter_ids(EDGES, Filter::gte("v", 1.0_f64)).await, ["a", "m", "n"]);
    assert_eq!(filter_ids(EDGES, Filter::in_list("v", [2_i32])).await, ["m"]);
    assert_eq!(filter_ids(EDGES, Filter::not_in_list("v", [2_i32])).await, ["a", "n", "s", "z"]);
    assert_eq!(filter_ids(EDGES, Filter::contains("v", "5")).await, ["s"]);

    let sorted = WasiProvider
        .query(
            EDGES,
            QueryOptions {
                order_by: vec![SortField {
                    field: "v".to_owned(),
                    descending: false,
                }],
                ..QueryOptions::default()
            },
        )
        .await
        .expect("query sorted edges");
    assert_eq!(ids(&sorted.documents), ["z", "m", "a", "n", "s"]);
}

async fn filter_ids(collection: &str, filter: Filter) -> Vec<String> {
    let page = WasiProvider
        .query(
            collection,
            QueryOptions {
                filter: Some(filter),
                ..QueryOptions::default()
            },
        )
        .await
        .expect("query filtered");
    page.documents.into_iter().map(|doc| doc.id).collect()
}

async fn insert(id: &str, body: Value) {
    WasiProvider
        .insert(ITEMS, &document(id, &body))
        .await
        .unwrap_or_else(|e| panic!("insert {id}: {e}"));
}

async fn put(id: &str, body: Value) {
    WasiProvider.put(ITEMS, &document(id, &body)).await.unwrap_or_else(|e| panic!("put {id}: {e}"));
}

async fn get(id: &str) -> Option<Value> {
    let doc = WasiProvider.get(ITEMS, id).await.unwrap_or_else(|e| panic!("get {id}: {e}"))?;
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
