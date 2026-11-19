//! `Filter::on_date` across calendar edges, observed through
//! `Filter::matches`, and `Filter::check` at and past each limit and on each
//! value it refuses.

use std::iter;

use omnia_sdk::Error;
use omnia_sdk::docstore::{ComparisonOp, Filter, ScalarValue};
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
        let err = Filter::on_date("at", date).expect_err(date);
        assert!(
            matches!(&err, Error::ServerError { code, .. } if code == "invalid"),
            "{date}: {err}"
        );
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
    assert!(matches!(&err, Error::ServerError { code, .. } if code == "invalid"), "{err}");
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

fn conditions(count: usize) -> Filter {
    Filter::and(iter::repeat_n(Filter::eq("rank", 1), count))
}

#[test]
fn condition_limit() {
    conditions(Filter::MAX_CONDITIONS).check().expect("a filter at the condition limit");
    refused(&conditions(Filter::MAX_CONDITIONS + 1));

    // an `or` of two `and`s distributes to a pair per two of their conditions
    let product = |left, right| Filter::or([conditions(left), conditions(right)]);
    product(5, 10).check().expect("a filter at the limit once distributed");
    refused(&product(6, 10));

    // a list that holds `null` counts twice
    let null_or_one = || Filter::in_list("rank", [ScalarValue::Null, ScalarValue::Int32(1)]);
    let lists = |count| Filter::and(iter::repeat_with(null_or_one).take(count));
    lists(Filter::MAX_CONDITIONS / 2).check().expect("lists at the condition limit");
    refused(&lists(Filter::MAX_CONDITIONS / 2 + 1));
}

#[test]
fn not_finite() {
    for number in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        refused(&Filter::eq("rank", number));
    }
}

#[test]
fn unordered_value() {
    for value in [ScalarValue::Null, ScalarValue::Bool(true)] {
        for op in [ComparisonOp::Lt, ComparisonOp::Lte, ComparisonOp::Gt, ComparisonOp::Gte] {
            refused(&Filter::cmp("rank", op, value.clone()));
        }
    }
    Filter::eq("rank", ScalarValue::Null).check().expect("an equality with `null`");
    Filter::ne("rank", true).check().expect("an inequality with a boolean");
}
