//! The `omnia_test::contract::document` rows over `WasiProvider`, all at
//! once, so the host's `omnia:docstore` and its backend meet the
//! `DocumentDb` contract the native doubles do. Filters at the limits then
//! pin the host's limits to the SDK's.

#![cfg(target_arch = "wasm32")]

use std::iter;

use omnia_sdk::docstore::{Document, Filter, Query};
use omnia_sdk::{DocumentDb as _, WasiProvider};
use omnia_test::contract::document;

omnia_sdk::api::command::export!(scenario);

const AT_LIMITS: &str = "atlimits";

async fn scenario() {
    let db = WasiProvider;

    // every row, all at once
    futures::join!(
        document::insert_exists(&db),
        document::portable_query(&db),
        document::filter_limits(&db),
        document::optional_query(&db),
    );

    // a filter at the depth and list limits is served
    db.put(AT_LIMITS, &Document::new("a", br#"{"rank":1}"#)).await.expect("put a document");
    let list = Filter::in_list("rank", (0_i32..).take(Filter::MAX_LIST_LEN));
    let nested = (1..Filter::MAX_DEPTH).fold(list, |inner, _| !inner);
    assert_eq!(selected(&db, nested).await, ["a"], "the filter at both limits selected others");

    // a filter at the condition limit is served
    let many = Filter::and(iter::repeat_n(Filter::eq("rank", 1), Filter::MAX_CONDITIONS));
    assert_eq!(
        selected(&db, many).await,
        ["a"],
        "the filter at the condition limit selected others"
    );
}

async fn selected(db: &WasiProvider, filter: Filter) -> Vec<String> {
    let query = Query {
        filter: Some(filter),
        ..Query::default()
    };
    let page = db.query(AT_LIMITS, &query).await.expect("a filter at a limit is served");
    page.items.into_iter().map(|document| document.id).collect()
}
