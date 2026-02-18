//! Integration tests for ORM filters.
//!
//! Tests the public API as users would interact with it.

#![cfg(target_arch = "wasm32")]
#![allow(missing_docs)]

mod common;

use common::{User, assert_sql_contains};
use qwasr_orm::{DataType, Filter, Join, SelectBuilder};

#[test]
fn filter_like_pattern() {
    let query = SelectBuilder::<User>::new()
        .r#where(Filter::like("name", "%john%".to_string()))
        .build()
        .unwrap();

    assert_sql_contains(&query.sql, &["WHERE", "users.name", "LIKE", "$1"]);
    assert_eq!(query.params.len(), 1);
    assert!(matches!(&query.params[0], DataType::Str(Some(s)) if s == "%john%"));
}

#[test]
fn filter_not_like_pattern() {
    let query = SelectBuilder::<User>::new()
        .r#where(Filter::not_like("name", "%admin%".to_string()))
        .build()
        .unwrap();

    assert_sql_contains(&query.sql, &["WHERE", "users.name", "NOT LIKE", "$1"]);
    assert_eq!(query.params.len(), 1);
    assert!(matches!(&query.params[0], DataType::Str(Some(s)) if s == "%admin%"));
}

#[test]
fn filter_between_values() {
    let query =
        SelectBuilder::<User>::new().r#where(Filter::between("id", 1, 100)).build().unwrap();

    assert_sql_contains(&query.sql, &["WHERE", "users.id", "BETWEEN", "$1", "AND", "$2"]);
    assert_eq!(query.params.len(), 2);
    assert!(matches!(query.params[0], DataType::Int32(Some(1))));
    assert!(matches!(query.params[1], DataType::Int32(Some(100))));
}

#[test]
fn filter_not_between_values() {
    let query =
        SelectBuilder::<User>::new().r#where(Filter::not_between("id", 10, 20)).build().unwrap();

    assert_sql_contains(&query.sql, &["WHERE", "users.id", "NOT BETWEEN", "$1", "AND", "$2"]);
    assert_eq!(query.params.len(), 2);
}

#[test]
fn filter_in_multiple_values() {
    let query = SelectBuilder::<User>::new()
        .r#where(Filter::r#in("id", vec![1, 2, 3, 4, 5]))
        .build()
        .unwrap();

    assert_sql_contains(&query.sql, &["WHERE", "users.id", "IN"]);
    assert_eq!(query.params.len(), 5);
}

#[test]
fn filter_in_empty_array() {
    let query = SelectBuilder::<User>::new()
        .r#where(Filter::r#in("id", Vec::<i32>::new()))
        .build()
        .unwrap();

    // NOTE: SeaQuery generates invalid SQL for empty IN clauses: WHERE ($1) = ($2)
    // This is a known limitation - callers should avoid empty IN arrays
    // or use Filter::And(vec![]) to short-circuit to no filter
    assert_sql_contains(&query.sql, &["WHERE", "($1)", "($2)"]);
    assert_eq!(query.params.len(), 2);
}

#[test]
fn filter_not_in_values() {
    let query =
        SelectBuilder::<User>::new().r#where(Filter::not_in("id", vec![99, 100])).build().unwrap();

    assert_sql_contains(&query.sql, &["WHERE", "users.id", "NOT IN"]);
    assert_eq!(query.params.len(), 2);
}

#[test]
fn filter_is_null() {
    let query = SelectBuilder::<User>::new().r#where(Filter::is_null("name")).build().unwrap();

    assert_sql_contains(&query.sql, &["WHERE", "users.name", "IS (NULL)"]);
    assert_eq!(query.params.len(), 0);
}

#[test]
fn filter_is_not_null() {
    let query = SelectBuilder::<User>::new().r#where(Filter::is_not_null("name")).build().unwrap();

    assert_sql_contains(&query.sql, &["WHERE", "users.name", "IS NOT (NULL)"]);
    assert_eq!(query.params.len(), 0);
}

#[test]
fn filter_any_values() {
    let query =
        SelectBuilder::<User>::new().r#where(Filter::any("id", vec![1, 2, 3])).build().unwrap();

    // ANY is implemented as IN for direct value arrays
    assert_sql_contains(&query.sql, &["WHERE", "users.id", "IN"]);
    assert_eq!(query.params.len(), 3);
}

#[test]
fn filter_table_qualified_comparison() {
    // Test table-qualified comparison (representative of all table_* variants)
    let query = SelectBuilder::<User>::new()
        .r#where(Filter::table_eq("users", "active", true))
        .build()
        .unwrap();

    assert_sql_contains(&query.sql, &["WHERE", "users.active", "=", "$1"]);
    assert!(matches!(query.params[0], DataType::Boolean(Some(true))));
}

#[test]
fn filter_table_qualified_is_null() {
    let query = SelectBuilder::<User>::new()
        .r#where(Filter::table_is_null("users", "name"))
        .build()
        .unwrap();

    assert_sql_contains(&query.sql, &["WHERE", "users.name", "IS (NULL)"]);
}

#[test]
fn filter_table_qualified_in() {
    let query = SelectBuilder::<User>::new()
        .r#where(Filter::table_in("users", "id", vec![1, 2, 3]))
        .build()
        .unwrap();

    assert_sql_contains(&query.sql, &["WHERE", "users.id", "IN"]);
    assert_eq!(query.params.len(), 3);
}

#[test]
fn filter_col_ne() {
    // Test column-to-column comparison (representative of all col_* variants)
    let query = SelectBuilder::<User>::new()
        .join(Join::left("user_roles", Filter::col_ne("users", "id", "user_roles", "user_id")))
        .build()
        .unwrap();

    assert_sql_contains(
        &query.sql,
        &["LEFT JOIN", "user_roles", "ON", "users.id", "<>", "user_roles.user_id"],
    );
}

#[test]
fn filter_nested_and_or() {
    let query = SelectBuilder::<User>::new()
        .r#where(Filter::And(vec![
            Filter::Or(vec![Filter::eq("active", true), Filter::eq("id", 1)]),
            Filter::gt("id", 0),
        ]))
        .build()
        .unwrap();

    // Should have nested boolean logic
    assert_sql_contains(&query.sql, &["WHERE"]);
    assert!(query.params.len() >= 2);
}

#[test]
fn filter_deeply_nested() {
    let query = SelectBuilder::<User>::new()
        .r#where(Filter::Or(vec![
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
    let query = SelectBuilder::<User>::new().r#where(Filter::And(vec![])).build().unwrap();

    // Empty AND should be treated as true (all conditions satisfied)
    assert_sql_contains(&query.sql, &["SELECT", "FROM users"]);
}

#[test]
fn filter_empty_or() {
    let query = SelectBuilder::<User>::new().r#where(Filter::Or(vec![])).build().unwrap();

    // Empty OR should be treated as false (no conditions satisfied)
    assert_sql_contains(&query.sql, &["SELECT", "FROM users"]);
}
