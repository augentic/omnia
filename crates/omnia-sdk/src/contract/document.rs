//! The [`DocumentDb`] rows.
//!
//! Each row uses a collection of its own, so the rows run in any order, or
//! all at once, over one store. Every collection name, id, body, and field
//! the rows use is portable, as the
//! [document store reference](https://github.com/augentic/omnia/blob/main/docs/reference/docstore.md#portable-names-and-bodies)
//! defines it. The store must start with the rows' collections empty.

use serde_json::{Value, json};

use super::expect_kind;
use crate::DocumentDb;
use crate::capability::{ErrorKind, Result};
use crate::docstore::{Document, Exists, Filter, Query, ScalarValue, SortField};

const EXISTS: &str = "insertexists";
const PORTABLE: &str = "portablequery";
const OPTIONAL: &str = "optionalquery";

// a cursor still running after this many pages never ends
const MAX_PAGES: usize = 32;

/// [`insert`](DocumentDb::insert) on an id already stored returns [`Exists`]
/// and leaves the stored body unchanged.
///
/// # Panics
///
/// Panics if the second insert fails, succeeds, or replaces the body.
pub async fn insert_exists(db: &impl DocumentDb) {
    let first = db.insert(EXISTS, &document("dup", &json!({"v": 1}))).await;
    assert_eq!(first.expect("insert a fresh id"), Ok(()), "a fresh id was refused");

    let second = db.insert(EXISTS, &document("dup", &json!({"v": 2}))).await;

    assert_eq!(second.expect("insert a stored id"), Err(Exists));
    let stored = db.get(EXISTS, "dup").await.expect("get").expect("the first body is stored");
    let body: Value = stored.decode().expect("a JSON body");
    assert_eq!(body, json!({"v": 1}), "a refused insert replaced the body");
}

/// Every portable filter selects what [`Filter::matches`] selects, a small
/// `limit` pages through each match once, and a zero `limit` fails with
/// [`ErrorKind::Invalid`].
///
/// # Panics
///
/// Panics if a filter selects other documents, a page holds more than its
/// `limit`, the cursor repeats or skips a match, or a zero `limit` does not
/// fail with [`ErrorKind::Invalid`].
pub async fn portable_query(db: &impl DocumentDb) {
    let stored = [
        ("a", json!({"kind": "fruit", "rank": 3, "weight": 0.5, "fresh": true})),
        ("b", json!({"kind": "veg", "rank": 1, "weight": 2.5, "fresh": false})),
        ("c", json!({"kind": "fruit", "rank": 2, "weight": 1.0, "fresh": false})),
        ("d", json!({"kind": "grain", "rank": 5, "weight": 4.0, "fresh": true})),
        ("e", json!({"kind": "veg", "rank": 4, "weight": 1.5, "fresh": true})),
    ];
    seed(db, PORTABLE, &stored).await;

    for filter in [
        Filter::eq("kind", "fruit"),
        Filter::ne("kind", "fruit"),
        Filter::gt("rank", 2),
        Filter::gte("weight", 1.5),
        Filter::lt("rank", 3),
        Filter::lte("weight", 1.0),
        Filter::eq("fresh", true),
        Filter::in_list("kind", ["veg", "grain"]),
        Filter::not_in_list("rank", [1, 5]),
        Filter::and([Filter::eq("fresh", true), Filter::gt("rank", 3)]),
        Filter::or([Filter::eq("kind", "grain"), Filter::lt("weight", 1.0)]),
        !Filter::eq("kind", "veg"),
    ] {
        let selected = walk(db, PORTABLE, &filtered(&filter)).await;
        let selected = selected.unwrap_or_else(|err| panic!("{filter:?} failed with `{err}`"));
        assert_eq!(sorted(selected), matching(&stored, &filter), "{filter:?} selected others");
    }

    let paged = Query {
        limit: Some(2),
        ..filtered(&Filter::gte("rank", 2))
    };
    let walked = walk(db, PORTABLE, &paged).await.expect("page through the matches");
    let expected = matching(&stored, &Filter::gte("rank", 2));
    assert_eq!(sorted(walked), expected, "the pages did not hold each match once");

    let zero = Query {
        limit: Some(0),
        ..Query::default()
    };
    expect_kind(db.query(PORTABLE, &zero).await, ErrorKind::Invalid, "a query with a zero limit");
}

/// Each optional query feature returns what [`Filter::matches`] and
/// [`SortField::compare`] define, or fails with [`ErrorKind::Unsupported`].
///
/// The optional features are:
///
/// - the string predicates;
/// - the null predicates, and a comparison with `null`;
/// - a dotted field path;
/// - `order_by`;
/// - `offset`.
///
/// # Panics
///
/// Panics if a feature selects or orders other documents, or fails with
/// another kind.
pub async fn optional_query(db: &impl DocumentDb) {
    let stored = [
        ("a", json!({"name": "apple", "rank": 3, "tag": "x", "meta": {"level": 1}})),
        ("b", json!({"name": "banana", "rank": 1, "tag": null, "meta": {"level": 2}})),
        ("c", json!({"name": "cherry", "rank": 2, "meta": {"level": 1}})),
        ("d", json!({"name": "date", "rank": 2, "tag": "y", "meta": {"level": 3}})),
    ];
    seed(db, OPTIONAL, &stored).await;

    for filter in [
        Filter::contains("name", "an"),
        Filter::starts_with("name", "ch"),
        Filter::ends_with("name", "e"),
        Filter::is_null("tag"),
        Filter::is_not_null("tag"),
        Filter::eq("tag", ScalarValue::Null),
        Filter::ne("tag", ScalarValue::Null),
        Filter::eq("meta.level", 1),
    ] {
        let selected = walk(db, OPTIONAL, &filtered(&filter)).await.map(sorted);
        served(selected, &matching(&stored, &filter), &format!("{filter:?}"));
    }

    for descending in [false, true] {
        let sort = SortField {
            field: "rank".to_owned(),
            descending,
        };
        let mut expected: Vec<_> = stored.iter().collect();
        expected.sort_by(|(a_id, a), (b_id, b)| sort.compare(a, b).then_with(|| a_id.cmp(b_id)));
        let expected: Vec<String> = expected.into_iter().map(|(id, _)| (*id).to_owned()).collect();
        let query = Query {
            order_by: vec![sort.clone()],
            ..Query::default()
        };
        served(walk(db, OPTIONAL, &query).await, &expected, &format!("{sort:?}"));
    }

    let unpaged = walk(db, OPTIONAL, &Query::default()).await.expect("query every document");
    let offset = Query {
        offset: Some(1),
        ..Query::default()
    };
    served(walk(db, OPTIONAL, &offset).await, &unpaged[1..], "an offset of 1");
}

// an optional feature is served as the reference defines it, or refused
#[track_caller]
fn served(selected: Result<Vec<String>>, expected: &[String], feature: &str) {
    match selected {
        Ok(ids) => assert_eq!(ids, expected, "{feature} returned other documents"),
        Err(err) => assert_eq!(err.kind(), ErrorKind::Unsupported, "{feature} failed with `{err}`"),
    }
}

async fn seed(db: &impl DocumentDb, collection: &str, stored: &[(&str, Value)]) {
    for (id, body) in stored {
        db.put(collection, &document(id, body))
            .await
            .unwrap_or_else(|err| panic!("put {id}: {err}"));
    }
}

// the ids `query` selects, page by page, in the order the pages hold them
async fn walk(db: &impl DocumentDb, collection: &str, query: &Query) -> Result<Vec<String>> {
    let mut query = query.clone();
    let mut ids = Vec::new();
    for _ in 0..MAX_PAGES {
        let page = db.query(collection, &query).await?;
        if let Some(limit) = query.limit {
            let held = u32::try_from(page.items.len()).unwrap_or(u32::MAX);
            assert!(held <= limit, "a page held {held} documents over a limit of {limit}");
        }
        ids.extend(page.items.into_iter().map(|document| document.id));
        match page.next {
            Some(next) => query.after = Some(next),
            None => return Ok(ids),
        }
    }
    panic!("the cursor did not end within {MAX_PAGES} pages");
}

// `stored` lists its documents in id order, so the ids come back sorted
fn matching(stored: &[(&str, Value)], filter: &Filter) -> Vec<String> {
    stored.iter().filter(|(_, body)| filter.matches(body)).map(|(id, _)| (*id).to_owned()).collect()
}

fn sorted(mut ids: Vec<String>) -> Vec<String> {
    ids.sort();
    ids
}

fn filtered(filter: &Filter) -> Query {
    Query {
        filter: Some(filter.clone()),
        ..Query::default()
    }
}

fn document(id: &str, body: &Value) -> Document {
    Document::json(id, body).expect("a JSON body")
}
