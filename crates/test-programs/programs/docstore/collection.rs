//! Two collections walked end to end over the raw `omnia:docstore` bindings,
//! each on the one handle `store::open` returns.
//!
//! - A write returns the revision a read reports, and a new body a new one.
//! - An `absent` write creates a document once, and a second create
//!   conflicts with the stored document.
//! - The batch calls answer in request order, and an id given twice is
//!   `invalid`.
//! - A delete answers whether it removed a document.
//! - A filter's groups read as `a and (b or c)`.
//! - A small `limit` walks every match once, even past a document deleted
//!   from an earlier page, and a page holds at most 1,000 documents.
//! - Malformed bodies, filters, limits, and cursors are `invalid`.
//! - The calls the backend trait cannot express yet are `unsupported`.

#![cfg(target_arch = "wasm32")]

use std::iter;

use omnia_sdk::docstore::collection::{Collection, Condition, Filter, SortField, Test, Value};
use omnia_sdk::docstore::store;
use omnia_sdk::docstore::types::{Conflict, Error, Precondition, Revision};
use serde_json::json;

omnia_sdk::command::export!(scenario);

// a cursor still running after this many pages never ends
const MAX_PAGES: usize = 8;

async fn scenario() {
    let docs = store::open("docs".to_owned()).await.expect("open docs");
    let query = store::open("query".to_owned()).await.expect("open query");

    // a write returns the revision a read reports, and a new body a new one
    let first = put(&docs, "a", &json!({"kind": "fruit", "rank": 3}), None).await;
    let first = first.expect("unconditional");
    let stored = docs.get("a".to_owned()).await.expect("get").expect("a document");
    assert_eq!(body(&stored.data), json!({"kind": "fruit", "rank": 3}));
    assert_eq!(stored.revision, first);
    let second = put(&docs, "a", &json!({"kind": "fruit", "rank": 4}), None).await;
    let second = second.expect("unconditional");
    assert_ne!(second, first);
    assert!(docs.get("missing".to_owned()).await.expect("get").is_none());

    // `absent` creates a document once
    let lock = Some(Precondition::Absent);
    let held = put(&docs, "lock", &json!({"held": true}), lock.clone()).await.expect("a create");
    let found = put(&docs, "lock", &json!({"held": false}), lock).await;
    let current = found.expect_err("a second create").current.expect("the stored document");
    assert_eq!(current.id, "lock");
    assert_eq!(body(&current.data), json!({"held": true}));
    assert_eq!(current.revision, held);

    // the batch calls answer in request order
    let pair = vec![entry("b", &json!({"n": 1})), entry("c", &json!({"n": 2}))];
    docs.put_many(pair).await.expect("put-many");
    let many = docs.get_many(ids(&["c", "missing", "b"])).await.expect("get-many");
    let many: Vec<_> = many.into_iter().map(|found| found.map(|document| document.id)).collect();
    assert_eq!(many, [Some("c".to_owned()), None, Some("b".to_owned())]);
    assert!(docs.exists("b".to_owned()).await.expect("exists"));
    docs.delete_many(ids(&["b", "missing"])).await.expect("delete-many");
    assert!(!docs.exists("b".to_owned()).await.expect("exists"));
    assert!(docs.exists("c".to_owned()).await.expect("exists"));

    // deleting an absent document succeeds and removes nothing
    let removed = docs.delete("b".to_owned(), None).await.expect("delete absent");
    assert!(!removed.expect("unconditional"));

    // a filter's groups read as `a and (b or c)`
    for (id, stored) in [
        ("q1", json!({"kind": "fruit", "rank": 1, "tag": "x"})),
        ("q2", json!({"kind": "fruit", "rank": 2})),
        ("q3", json!({"kind": "veg", "rank": 3, "tag": null})),
        ("q4", json!({"kind": "veg", "rank": 4, "tag": "y"})),
        ("q5", json!({"kind": "grain", "rank": 5, "tag": "x"})),
    ] {
        put(&query, id, &stored, None).await.expect("unconditional");
    }
    let grouped = vec![
        vec![test("rank", Test::Gt(Value::Integer(1)))],
        vec![test("kind", Test::Eq(text("fruit"))), test("tag", Test::Eq(text("x")))],
    ];
    assert_eq!(selected(&query, grouped).await, ["q2", "q5"]);
    let not_veg = vec![vec![negated("kind", Test::Eq(text("veg")))]];
    assert_eq!(selected(&query, not_veg).await, ["q1", "q2", "q5"]);
    let null = vec![vec![test("tag", Test::Null)]];
    assert_eq!(selected(&query, null).await, ["q2", "q3"]);
    let not_null = vec![vec![negated("tag", Test::Null)]];
    assert_eq!(selected(&query, not_null).await, ["q1", "q4", "q5"]);
    let listed = vec![vec![test("rank", Test::In(integers(&[1, 4, 9])))]];
    assert_eq!(selected(&query, listed).await, ["q1", "q4"]);
    assert_eq!(selected(&query, Vec::new()).await, ["q1", "q2", "q3", "q4", "q5"]);

    // a limit of 2 walks every match once
    let by_rank = vec![SortField {
        field: "rank".to_owned(),
        descending: false,
    }];
    let walked = walk(&query, &by_rank, None).await;
    assert_eq!(walked, ["q1", "q2", "q3", "q4", "q5"]);

    // a document deleted from the first page shifts none of the rest
    let page = query.query(Vec::new(), by_rank.clone(), None, Some(2)).await.expect("query");
    let first: Vec<&str> = page.documents.iter().map(|document| document.id.as_str()).collect();
    assert_eq!(first, ["q1", "q2"]);
    let removed = query.delete("q1".to_owned(), None).await.expect("delete");
    assert!(removed.expect("unconditional"));
    let rest = walk(&query, &by_rank, page.next).await;
    assert_eq!(rest, ["q3", "q4", "q5"]);

    // with no limit, a page stops at 1,000 documents
    let bulk = (0..1001).map(|n| entry(&format!("n{n:04}"), &json!({"n": n}))).collect();
    query.put_many(bulk).await.expect("put-many");
    let full = query.query(Vec::new(), Vec::new(), None, None).await.expect("query");
    assert_eq!(full.documents.len(), 1000);
    let after = full.next.expect("a second page");
    let tail = query.query(Vec::new(), Vec::new(), Some(after), None).await.expect("query");
    assert_eq!(tail.next, None);
    assert_eq!(full.documents.len() + tail.documents.len(), 1005);

    // refused as `invalid`
    let raw = docs.put("raw".to_owned(), "not json".to_owned(), None).await;
    assert!(matches!(raw, Err(Error::Invalid(_))), "{raw:?}");
    let array = docs.put("array".to_owned(), "[1]".to_owned(), None).await;
    assert!(matches!(array, Err(Error::Invalid(_))), "{array:?}");
    let twice = vec![entry("twice", &json!({"n": 1})), entry("twice", &json!({"n": 2}))];
    let twice = docs.put_many(twice).await;
    assert!(matches!(twice, Err(Error::Invalid(_))), "{twice:?}");
    let zero = query.query(Vec::new(), Vec::new(), None, Some(0)).await;
    assert!(matches!(zero, Err(Error::Invalid(_))), "{zero:?}");
    let forged = query.query(Vec::new(), Vec::new(), Some("forged".to_owned()), None).await;
    assert!(matches!(forged, Err(Error::Invalid(_))), "{forged:?}");
    let many = iter::repeat_n(vec![test("rank", Test::Eq(Value::Integer(1)))], 101).collect();
    for (filter, refused) in [
        (vec![Vec::new()], "an empty group"),
        (many, "101 conditions"),
        (vec![vec![test("rank", Test::In(Vec::new()))]], "an `in` of no values"),
        (vec![vec![test("rank", Test::In(integers(&[1; 101])))]], "an `in` of 101 values"),
        (vec![vec![test("rank", Test::Lt(Value::Boolean(true)))]], "an ordered boolean"),
        (vec![vec![test("rank", Test::Gt(Value::Number(f64::NAN)))]], "a NaN"),
    ] {
        let served = query.query(filter, Vec::new(), None, None).await;
        assert!(matches!(served, Err(Error::Invalid(_))), "{refused}: {served:?}");
    }

    // refused as `unsupported` until backends carry revisions and arrays
    let matching = Some(Precondition::Matches(second.clone()));
    let replace = docs.put("a".to_owned(), "{}".to_owned(), matching).await;
    assert!(matches!(replace, Err(Error::Unsupported(_))), "{replace:?}");
    let delete = docs.delete("a".to_owned(), Some(second)).await;
    assert!(matches!(delete, Err(Error::Unsupported(_))), "{delete:?}");
    let elements = vec![vec![test("tags", Test::ArrayContains(text("x")))]];
    let elements = query.query(elements, Vec::new(), None, None).await;
    assert!(matches!(elements, Err(Error::Unsupported(_))), "{elements:?}");
}

async fn put(
    collection: &Collection, id: &str, body: &serde_json::Value, when: Option<Precondition>,
) -> Result<Revision, Conflict> {
    let written = collection.put(id.to_owned(), body.to_string(), when).await;
    written.unwrap_or_else(|err| panic!("put `{id}`: {err:?}"))
}

// the ids every match of one unpaged query holds, sorted
async fn selected(collection: &Collection, filter: Filter) -> Vec<String> {
    let page = collection.query(filter, Vec::new(), None, None).await.expect("query");
    assert_eq!(page.next, None);
    let mut ids: Vec<String> = page.documents.into_iter().map(|document| document.id).collect();
    ids.sort();
    ids
}

// the ids pages of two hold from `after` on, in the order they hold them
async fn walk(
    collection: &Collection, order_by: &[SortField], after: Option<String>,
) -> Vec<String> {
    let mut after = after;
    let mut ids = Vec::new();
    for _ in 0..MAX_PAGES {
        let page = collection.query(Vec::new(), order_by.to_vec(), after, Some(2)).await;
        let page = page.expect("query");
        assert!(page.documents.len() <= 2, "a page held {} documents", page.documents.len());
        ids.extend(page.documents.into_iter().map(|document| document.id));
        match page.next {
            Some(next) => after = Some(next),
            None => return ids,
        }
    }
    panic!("the cursor did not end within {MAX_PAGES} pages");
}

fn test(field: &str, test: Test) -> Condition {
    Condition {
        field: field.to_owned(),
        test,
        negated: false,
    }
}

fn negated(field: &str, test: Test) -> Condition {
    Condition {
        field: field.to_owned(),
        test,
        negated: true,
    }
}

fn text(text: &str) -> Value {
    Value::Text(text.to_owned())
}

fn integers(integers: &[i64]) -> Vec<Value> {
    integers.iter().copied().map(Value::Integer).collect()
}

fn entry(id: &str, body: &serde_json::Value) -> (String, String) {
    (id.to_owned(), body.to_string())
}

fn ids(ids: &[&str]) -> Vec<String> {
    ids.iter().map(|&id| id.to_owned()).collect()
}

fn body(data: &str) -> serde_json::Value {
    serde_json::from_str(data).expect("a JSON body")
}
