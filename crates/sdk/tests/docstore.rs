//! `Filter::on_date` across calendar edges, observed through `Filter::matches`.

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
