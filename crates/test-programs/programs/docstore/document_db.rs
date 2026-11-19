//! The `omnia_test::contract::document` rows, all at once, so the host's
//! `omnia:docstore` and its backend meet the contract. Filters at the limits
//! then pin the host's limits to the SDK's.

#![cfg(target_arch = "wasm32")]

use std::iter;

use omnia_sdk::docstore::collection::Collection;
use omnia_sdk::docstore::{Filter, Query, store};
use omnia_test::contract::document;

omnia_sdk::api::command::export!(scenario);

const AT_LIMITS: &str = "atlimits";

async fn scenario() {
    // every row, all at once
    futures::join!(
        document::insert_exists(),
        document::portable_query(),
        document::filter_limits(),
        document::optional_query(),
    );

    // a filter at the depth and list limits is served
    let collection = store::open(AT_LIMITS.to_owned()).await.expect("open the collection");
    collection.upsert("a", &serde_json::json!({"rank": 1})).await.expect("upsert a document");
    let list = Filter::in_list("rank", (0_i32..).take(Filter::MAX_LIST_LEN));
    let nested = (1..Filter::MAX_DEPTH).fold(list, |inner, _| !inner);
    let at_both = selected(&collection, nested).await;
    assert_eq!(at_both, ["a"], "the filter at both limits selected others");

    // a filter at the condition limit is served
    let many = Filter::and(iter::repeat_n(Filter::eq("rank", 1), Filter::MAX_CONDITIONS));
    let at_conditions = selected(&collection, many).await;
    assert_eq!(at_conditions, ["a"], "the filter at the condition limit selected others");
}

async fn selected(collection: &Collection, filter: Filter) -> Vec<String> {
    let query = Query {
        filter: Some(filter),
        ..Query::default()
    };
    let page = collection.find(&query).await.expect("a filter at a limit is served");
    page.documents.into_iter().map(|document| document.id).collect()
}
