//! End-to-end walk over the `omnia:docstore` helpers of `omnia_sdk`.
//!
//! - Inserted bodies read back intact, and an unknown id reads as `None`.
//! - `upsert` overwrites an existing id and creates an absent one.
//! - Queries list in id order, filter, sort with type rank, and paginate.
//! - `remove` reports whether a document was removed.

#![cfg(target_arch = "wasm32")]

use omnia_sdk::docstore::{self, Collection, Document, Filter, Query, SortField};
use serde_json::{Value, json};

omnia_sdk::command::export!(scenario);

const ITEMS: &str = "items";
const EDGES: &str = "edges";

async fn scenario() {
    let items = docstore::open(ITEMS).await.expect("open items");
    let edges = docstore::open(EDGES).await.expect("open edges");

    // seed documents mixing present, explicit null, and absent fields
    insert(&items, "a", json!({"kind": "fruit", "rank": 3, "zone": "z1"})).await;
    insert(&items, "b", json!({"kind": "fruit", "rank": 1, "zone": null})).await;
    insert(&items, "c", json!({"kind": "veg", "rank": 2, "zone": "z2"})).await;
    insert(&items, "d", json!({"kind": "fruit", "rank": 2, "zone": "z3"})).await;
    insert(&items, "e", json!({"kind": "fruit", "rank": 5})).await;
    insert(&items, "g", json!({"kind": "fruit", "rank": 4, "zone": "z4"})).await;

    // point reads return the stored body or none
    assert_eq!(get(&items, "a").await, Some(json!({"kind": "fruit", "rank": 3, "zone": "z1"})));
    assert_eq!(get(&items, "missing").await, None);

    // put overwrites an existing id and creates an absent one
    upsert(&items, "c", json!({"kind": "veg", "rank": 9, "zone": "z2", "fresh": true})).await;
    assert_eq!(
        get(&items, "c").await,
        Some(json!({"kind": "veg", "rank": 9, "zone": "z2", "fresh": true}))
    );
    upsert(&items, "f", json!({"kind": "veg", "rank": 0})).await;
    assert_eq!(get(&items, "f").await, Some(json!({"kind": "veg", "rank": 0})));

    // unfiltered query lists every document in id order on one page
    let all = items.find(&Query::default()).await.expect("query all");
    assert_eq!(ids(&all.documents), ["a", "b", "c", "d", "e", "f", "g"]);
    assert_eq!(all.next, None);

    // fruit with a zone, highest rank first, two per page
    let page = Query {
        filter: Some(Filter::and([Filter::eq("kind", "fruit"), Filter::is_not_null("zone")])),
        order_by: vec![SortField::desc("rank")],
        limit: Some(2),
        ..Query::default()
    };
    let first = items.find(&page).await.expect("query first page");
    assert_eq!(ids(&first.documents), ["g", "a"]);
    let after = first.next.expect("more pages");
    let rest = Query {
        after: Some(after),
        ..page
    };
    let rest = items.find(&rest).await.expect("query last page");
    assert_eq!(ids(&rest.documents), ["d"]);
    assert_eq!(rest.next, None);

    // remove reports whether a document was removed
    assert!(remove(&items, "b").await);
    assert!(!remove(&items, "b").await);
    assert_eq!(get(&items, "b").await, None);

    // absent or incomparable fields across operators and ascending sort order
    for (id, body) in [
        ("a", json!({"v": 5})),
        ("m", json!({"v": 2.0})),
        ("n", json!({"v": 15})),
        ("s", json!({"v": "1.5"})),
        ("z", json!({})),
    ] {
        insert(&edges, id, body).await;
    }
    assert_eq!(filter_ids(&edges, Filter::gte("v", 1.0_f64)).await, ["a", "m", "n"]);
    assert_eq!(filter_ids(&edges, Filter::in_list("v", [2_i32])).await, ["m"]);
    assert_eq!(filter_ids(&edges, Filter::not_in_list("v", [2_i32])).await, ["a", "n", "s", "z"]);
    assert_eq!(filter_ids(&edges, Filter::contains("v", "5")).await, ["s"]);

    let sorted = Query {
        order_by: vec![SortField::asc("v")],
        ..Query::default()
    };
    let sorted = edges.find(&sorted).await.expect("query sorted edges");
    assert_eq!(ids(&sorted.documents), ["z", "m", "a", "n", "s"]);
}

async fn filter_ids(collection: &Collection, filter: Filter) -> Vec<String> {
    let query = Query {
        filter: Some(filter),
        ..Query::default()
    };
    let page = collection.find(&query).await.expect("query filtered");
    page.documents.into_iter().map(|doc| doc.id).collect()
}

async fn insert(collection: &Collection, id: &str, body: Value) {
    let inserted =
        collection.insert(id, &body).await.unwrap_or_else(|e| panic!("insert {id}: {e}"));
    assert_eq!(inserted, Ok(()), "{id} is a fresh id");
}

async fn upsert(collection: &Collection, id: &str, body: Value) {
    collection.upsert(id, &body).await.unwrap_or_else(|e| panic!("upsert {id}: {e}"));
}

async fn get(collection: &Collection, id: &str) -> Option<Value> {
    collection.get_as(id).await.unwrap_or_else(|e| panic!("get {id}: {e}"))
}

async fn remove(collection: &Collection, id: &str) -> bool {
    collection.remove(id).await.unwrap_or_else(|e| panic!("remove {id}: {e}"))
}

fn ids(documents: &[Document]) -> Vec<&str> {
    documents.iter().map(|doc| doc.id.as_str()).collect()
}
