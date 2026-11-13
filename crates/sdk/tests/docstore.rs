//! `Filter::on_date` across calendar edges, observed through
//! `Filter::matches`, and `Filter::check` at and past each limit.

use std::iter;

use omnia_sdk::capability::ErrorKind;
use omnia_sdk::docstore::Filter;
use serde_json::json;

fn on(date: &str, timestamp: &str) -> bool {
    Filter::on_date("at", date).expect("valid date").matches(&json!({ "at": timestamp }))
}

#[test]
fn leap_day() {
    assert!(on("2024-02-28", "2024-02-28T23:59:59Z"));
    assert!(!on("2024-02-28", "2024-02-29T00:00:00Z"));
    assert!(on("2024-02-29", "2024-02-29T12:00:00Z"));
    assert!(!on("2024-02-29", "2024-03-01T00:00:00Z"));
    assert!(on("2000-02-29", "2000-02-29T00:00:00Z"));
}

#[test]
fn year_end() {
    assert!(!on("2026-12-31", "2026-12-30T23:59:59Z"));
    assert!(on("2026-12-31", "2026-12-31T23:59:59Z"));
    assert!(!on("2026-12-31", "2027-01-01T00:00:00Z"));
}

#[test]
fn invalid_dates() {
    for date in [
        "2023-02-29",
        "1900-02-29",
        "2026-04-31",
        "2026-13-01",
        "2026-00-10",
        "2024-1-5",
        "2024-01-05T00:00:00Z",
        "",
        "9999-12-31",
    ] {
        Filter::on_date("at", date).expect_err(date);
    }
}

fn nested(depth: usize) -> Filter {
    (1..depth).fold(Filter::eq("rank", 1), |inner, _| !inner)
}

fn list(len: usize) -> Filter {
    Filter::in_list("rank", iter::repeat_n(1, len))
}

#[track_caller]
fn refused(filter: &Filter) {
    let err = filter.check().expect_err("a filter past a limit");
    assert_eq!(err.kind(), ErrorKind::Invalid, "{err}");
}

#[test]
fn depth_limit() {
    nested(Filter::MAX_DEPTH).check().expect("a filter at the depth limit");
    refused(&nested(Filter::MAX_DEPTH + 1));
    refused(&Filter::and([nested(Filter::MAX_DEPTH)]));
}

#[test]
fn list_limit() {
    for len in [1, Filter::MAX_LIST_LEN] {
        list(len).check().unwrap_or_else(|err| panic!("a list of {len} values: {err}"));
    }
    refused(&list(0));
    refused(&list(Filter::MAX_LIST_LEN + 1));
    refused(&Filter::not_in_list("rank", Vec::<i32>::new()));
}

#[test]
fn empty_combinator() {
    refused(&Filter::and([]));
    refused(&Filter::or([]));
    refused(&!Filter::and([Filter::eq("rank", 1), Filter::or([])]));
}
