//! Common test helpers shared across integration tests.
#![cfg(target_arch = "wasm32")]
#![allow(dead_code)]

use chrono::{DateTime, Utc};
use omnia_orm::{Filter, Join, entity};

// Common test entities used across multiple test files

entity! {
    table = "users",
    #[derive(Debug, Clone)]
    pub struct User {
        pub id: i64,
        pub name: String,
        pub active: bool,
    }
}

entity! {
    table = "posts",
    joins = [Join::left("users", Filter::col_eq("posts", "author_id", "users", "id"))],
    #[derive(Debug, Clone)]
    pub struct PostWithJoin {
        pub id: i64,
        pub title: String,
    }
}

entity! {
    table = "comments",
    columns = [("users", "name", "author_name")],
    joins = [Join::left("users", Filter::col_eq("comments", "user_id", "users", "id"))],
    #[derive(Debug, Clone)]
    pub struct CommentWithAlias {
        pub id: i64,
        pub content: String,
        pub author_name: String,
    }
}

entity! {
    table = "items",
    #[derive(Debug, Clone)]
    pub struct Item {
        pub id: i64,
        pub name: String,
        pub count: i32,
    }
}

entity! {
    table = "events",
    #[derive(Debug, Clone)]
    pub struct Event {
        pub id: i64,
        pub occurred_at: DateTime<Utc>,
    }
}

/// Normalize SQL by collapsing whitespace.
fn normalize_sql(sql: &str) -> String {
    sql.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Canonicalize SQL for comparison by removing identifier quotes and normalizing whitespace.
/// Preserves quotes inside string literals.
fn canonicalize_sql(sql: &str) -> String {
    let mut cleaned = String::with_capacity(sql.len());
    let mut in_single_quote = false;

    for ch in sql.chars() {
        match ch {
            '\'' => {
                in_single_quote = !in_single_quote;
                cleaned.push(ch);
            }
            '"' if !in_single_quote => {
                // Strip identifier quoting to avoid brittle comparisons.
            }
            _ => cleaned.push(ch),
        }
    }

    normalize_sql(&cleaned)
}

/// Assert that SQL contains all expected fragments in order.
///
/// This helper normalizes SQL to avoid brittle exact-string matching with ``SeaQuery`` output.
/// It strips identifier quotes, normalizes whitespace, and checks that fragments appear
/// sequentially in the generated SQL.
#[allow(clippy::missing_panics_doc)]
pub fn assert_sql_contains(actual: &str, fragments: &[&str]) {
    let actual_canonical = canonicalize_sql(actual);
    let mut search_start = 0usize;

    for fragment in fragments {
        let fragment_canonical = canonicalize_sql(fragment);
        if fragment_canonical.is_empty() {
            continue;
        }

        if let Some(pos) = actual_canonical[search_start..].find(&fragment_canonical) {
            search_start += pos + fragment_canonical.len();
        } else {
            use std::io::Write;
            let mut stderr = std::io::stderr();
            writeln!(stderr, "*** fragment-canonical: {fragment_canonical}").unwrap();
            writeln!(stderr, "*** actual-canonical-sql: {actual_canonical}").unwrap();
            stderr.flush().unwrap();

            panic!(
                "expected SQL fragment `{fragment_canonical}` not found in `{actual_canonical}`"
            );
        }
    }
}
