//! Integration tests for ORM filters.
//!
//! Tests the public API as users would interact with it.

#![cfg(target_arch = "wasm32")]
#![allow(missing_docs)]

mod common;

use common::{User, assert_sql_contains};
use omnia_orm::{DataType, Entity, Filter, Join};

#[test]
fn filter_like_pattern() {
    let query = User::select().filter(Filter::like("name", "%john%")).build().unwrap();

    assert_sql_contains(&query.sql, &["WHERE", "users.name", "LIKE", "$1"]);
    assert_eq!(query.params.len(), 1);
    assert!(matches!(&query.params[0], DataType::Str(Some(s)) if s == "%john%"));
}

#[test]
fn filter_in_multiple_values() {
    let query = User::select().filter(Filter::r#in("id", vec![1, 2, 3, 4, 5])).build().unwrap();

    assert_sql_contains(&query.sql, &["WHERE", "users.id", "IN"]);
    assert_eq!(query.params.len(), 5);
}

#[test]
fn filter_in_empty_array() {
    let query = User::select().filter(Filter::r#in("id", Vec::<i32>::new())).build().unwrap();

    // SeaQuery generates degenerate SQL for empty IN clauses
    assert_sql_contains(&query.sql, &["WHERE", "($1)", "($2)"]);
    assert_eq!(query.params.len(), 2);
}

#[test]
fn filter_is_null() {
    let query = User::select().filter(Filter::is_null("name")).build().unwrap();

    assert_sql_contains(&query.sql, &["WHERE", "users.name", "IS NULL"]);
    assert_eq!(query.params.len(), 0);
}

#[test]
fn filter_is_not_null() {
    let query = User::select().filter(Filter::is_not_null("name")).build().unwrap();

    assert_sql_contains(&query.sql, &["WHERE", "users.name", "IS NOT NULL"]);
    assert_eq!(query.params.len(), 0);
}

#[test]
fn filter_table_qualified_via_on() {
    let query = User::select().filter(Filter::eq("active", true).on("users")).build().unwrap();

    assert_sql_contains(&query.sql, &["WHERE", "users.active", "=", "$1"]);
    assert!(matches!(query.params[0], DataType::Boolean(Some(true))));
}

#[test]
fn filter_table_qualified_is_null_via_on() {
    let query = User::select().filter(Filter::is_null("name").on("users")).build().unwrap();

    assert_sql_contains(&query.sql, &["WHERE", "users.name", "IS NULL"]);
}

#[test]
fn filter_table_qualified_in_via_on() {
    let query =
        User::select().filter(Filter::r#in("id", vec![1, 2, 3]).on("users")).build().unwrap();

    assert_sql_contains(&query.sql, &["WHERE", "users.id", "IN"]);
    assert_eq!(query.params.len(), 3);
}

#[test]
fn filter_col_eq_in_join() {
    let query = User::select()
        .join(Join::left("user_roles", Filter::col_eq("users", "id", "user_roles", "user_id")))
        .build()
        .unwrap();

    assert_sql_contains(
        &query.sql,
        &["LEFT JOIN", "user_roles", "ON", "users.id", "=", "user_roles.user_id"],
    );
}

#[test]
fn filter_nested_and_or() {
    let query = User::select()
        .filter(Filter::And(vec![
            Filter::Or(vec![Filter::eq("active", true), Filter::eq("id", 1)]),
            Filter::gt("id", 0),
        ]))
        .build()
        .unwrap();

    assert_sql_contains(&query.sql, &["WHERE"]);
    assert!(query.params.len() >= 2);
}

#[test]
fn filter_deeply_nested() {
    let query = User::select()
        .filter(Filter::Or(vec![
            Filter::And(vec![Filter::eq("active", true), Filter::gt("id", 10)]),
            Filter::And(vec![Filter::eq("active", false), Filter::lt("id", 5)]),
        ]))
        .build()
        .unwrap();

    assert_sql_contains(&query.sql, &["WHERE", "AND", "OR"]);
    assert_eq!(query.params.len(), 4);
    assert!(matches!(query.params[0], DataType::Boolean(Some(true))));
    assert!(matches!(query.params[1], DataType::Int32(Some(10))));
    assert!(matches!(query.params[2], DataType::Boolean(Some(false))));
    assert!(matches!(query.params[3], DataType::Int32(Some(5))));
}

#[test]
fn filter_empty_and() {
    let query = User::select().filter(Filter::And(vec![])).build().unwrap();

    assert_sql_contains(&query.sql, &["SELECT", "FROM users"]);
}

#[test]
fn filter_empty_or() {
    let query = User::select().filter(Filter::Or(vec![])).build().unwrap();

    assert_sql_contains(&query.sql, &["SELECT", "FROM users"]);
}
