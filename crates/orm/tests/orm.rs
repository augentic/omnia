//! Integration tests for ORM query builders.
//!
//! Tests the public API as users would interact with it.

#![cfg(target_arch = "wasm32")]
#![allow(missing_docs)]

mod common;

use common::{Item, User, assert_sql_contains};
use omnia_orm::{DeleteBuilder, Entity, Filter, InsertBuilder, Join, Order, UpdateBuilder};
use omnia_wasi_sql::types::DataType;

// SELECT tests

#[test]
fn select_basic() {
    let query = User::select().build().unwrap();
    assert_sql_contains(
        &query.sql,
        &["SELECT", "users.id", "users.name", "users.active", "FROM users"],
    );
    assert_eq!(query.params.len(), 0);
}

#[test]
fn select_with_ordering_and_limits() {
    let query = User::select()
        .filter(Filter::eq("active", true))
        .filter(Filter::gt("id", 100))
        .order_by("id", Order::Asc)
        .order_by("name", Order::Desc)
        .limit(10)
        .offset(5)
        .build()
        .unwrap();

    assert_sql_contains(
        &query.sql,
        &[
            "SELECT",
            "users.id",
            "users.name",
            "users.active",
            "WHERE",
            "users.active",
            "=",
            "$1",
            "AND",
            "users.id",
            ">",
            "$2",
            "ORDER BY",
            "users.id",
            "ASC",
            "users.name",
            "DESC",
            "LIMIT $3",
            "OFFSET $4",
        ],
    );

    assert_eq!(query.params.len(), 4);
    assert!(matches!(query.params[0], DataType::Boolean(Some(true))));
    assert!(matches!(query.params[1], DataType::Int32(Some(100))));
    assert!(matches!(query.params[2], DataType::Uint64(Some(10))));
    assert!(matches!(query.params[3], DataType::Uint64(Some(5))));
}

#[test]
fn select_with_column_aliasing() {
    let query = User::select()
        .column_as("roles.name", "role_name")
        .join(Join::left("roles", Filter::col_eq("users", "role_id", "roles", "id")))
        .build()
        .unwrap();

    assert_sql_contains(
        &query.sql,
        &[
            "SELECT",
            "users.id",
            "users.name",
            "users.active",
            "roles.name",
            "AS",
            "role_name",
            "FROM users",
            "LEFT JOIN roles",
        ],
    );
}

#[test]
fn select_with_ad_hoc_join() {
    let query = User::select()
        .join(Join::inner("user_roles", Filter::col_eq("users", "id", "user_roles", "user_id")))
        .build()
        .unwrap();

    assert_sql_contains(
        &query.sql,
        &["SELECT", "users.id", "INNER JOIN user_roles ON", "users.id", "=", "user_roles.user_id"],
    );
}

#[test]
fn select_with_or_filter() {
    let query = User::select()
        .filter(Filter::Or(vec![Filter::eq("active", true), Filter::gt("id", 100)]))
        .build()
        .unwrap();

    assert_sql_contains(
        &query.sql,
        &["SELECT", "users.id", "WHERE", "users.active", "=", "$1", "OR", "users.id", ">", "$2"],
    );
}

#[test]
fn select_with_not_filter() {
    let query =
        User::select().filter(Filter::Not(Box::new(Filter::eq("active", false)))).build().unwrap();

    assert_sql_contains(
        &query.sql,
        &["SELECT", "users.id", "WHERE", "NOT", "users.active", "=", "$1"],
    );
}

#[test]
fn select_with_right_join() {
    let query = User::select()
        .join(Join::right("profiles", Filter::col_eq("users", "id", "profiles", "user_id")))
        .build()
        .unwrap();

    assert_sql_contains(&query.sql, &["SELECT", "users.id", "FROM users", "RIGHT JOIN profiles"]);
}

#[test]
fn select_with_full_join() {
    let query = User::select()
        .join(Join::full("accounts", Filter::col_eq("users", "id", "accounts", "user_id")))
        .build()
        .unwrap();

    assert_sql_contains(
        &query.sql,
        &["SELECT", "users.id", "FROM users", "FULL OUTER JOIN accounts"],
    );
}

#[test]
fn select_with_multiple_join_types() {
    let query = User::select()
        .join(Join::inner("roles", Filter::col_eq("users", "role_id", "roles", "id")))
        .join(Join::left("profiles", Filter::col_eq("users", "id", "profiles", "user_id")))
        .join(Join::right("sessions", Filter::col_eq("users", "id", "sessions", "user_id")))
        .build()
        .unwrap();

    assert_sql_contains(
        &query.sql,
        &["FROM users", "INNER JOIN roles", "LEFT JOIN profiles", "RIGHT JOIN sessions"],
    );
}

// INSERT tests

#[test]
fn insert_basic() {
    let query = InsertBuilder::new("items").set("name", "test").set("count", 42).build().unwrap();

    assert_sql_contains(&query.sql, &["INSERT INTO items", "name", "count", "VALUES", "$1", "$2"]);

    assert_eq!(query.params.len(), 2);
    assert!(matches!(&query.params[0], DataType::Str(Some(s)) if s == "test"));
    assert!(matches!(query.params[1], DataType::Int32(Some(42))));
}

#[test]
fn insert_from_entity() {
    let item = Item {
        id: 1,
        name: "test".to_string(),
        count: 10,
    };

    let query = InsertBuilder::from(&item).build().unwrap();

    assert_sql_contains(
        &query.sql,
        &["INSERT INTO items", "id", "name", "count", "VALUES", "$1", "$2", "$3"],
    );

    assert_eq!(query.params.len(), 3);
    assert!(matches!(query.params[0], DataType::Int64(Some(1))));
    assert!(matches!(&query.params[1], DataType::Str(Some(s)) if s == "test"));
    assert!(matches!(query.params[2], DataType::Int32(Some(10))));
}

#[test]
fn insert_via_entity_convenience() {
    let item = Item {
        id: 1,
        name: "test".to_string(),
        count: 10,
    };

    let query = item.insert().build().unwrap();

    assert_sql_contains(
        &query.sql,
        &["INSERT INTO items", "id", "name", "count", "VALUES", "$1", "$2", "$3"],
    );
    assert_eq!(query.params.len(), 3);
}

// UPDATE tests

#[test]
fn update_basic() {
    let query = UpdateBuilder::new("items")
        .set("name", "updated")
        .filter(Filter::eq("id", 1))
        .build()
        .unwrap();

    assert_sql_contains(
        &query.sql,
        &["UPDATE items", "SET name = $1", "WHERE", "items.id", "=", "$2"],
    );

    assert_eq!(query.params.len(), 2);
    assert!(matches!(&query.params[0], DataType::Str(Some(s)) if s == "updated"));
    assert!(matches!(query.params[1], DataType::Int32(Some(1))));
}

#[test]
fn update_multiple_fields() {
    let query = UpdateBuilder::new("items")
        .set("name", "new")
        .set("id", 99)
        .filter(Filter::eq("id", 1))
        .build()
        .unwrap();

    assert_sql_contains(
        &query.sql,
        &["UPDATE items", "SET name = $1, id = $2", "WHERE", "items.id", "=", "$3"],
    );

    assert_eq!(query.params.len(), 3);
    assert!(matches!(&query.params[0], DataType::Str(Some(s)) if s == "new"));
    assert!(matches!(query.params[1], DataType::Int32(Some(99))));
    assert!(matches!(query.params[2], DataType::Int32(Some(1))));
}

#[test]
fn update_no_filter() {
    let query = UpdateBuilder::new("items").set("name", "global").build().unwrap();

    assert_sql_contains(&query.sql, &["UPDATE items", "SET name = $1"]);

    assert_eq!(query.params.len(), 1);
    assert!(matches!(&query.params[0], DataType::Str(Some(s)) if s == "global"));
}

#[test]
fn update_set_if_some() {
    let query = UpdateBuilder::new("items")
        .set_if("name", Some("updated"))
        .set_if("count", None::<i32>)
        .filter(Filter::eq("id", 1))
        .build()
        .unwrap();

    assert_sql_contains(
        &query.sql,
        &["UPDATE items", "SET name = $1", "WHERE", "items.id", "=", "$2"],
    );
    assert_eq!(query.params.len(), 2);
    assert!(matches!(&query.params[0], DataType::Str(Some(s)) if s == "updated"));
}

// DELETE tests

#[test]
fn delete_with_filter() {
    let query = Item::delete().filter(Filter::eq("id", 1)).build().unwrap();

    assert_sql_contains(&query.sql, &["DELETE FROM items", "WHERE", "items.id", "=", "$1"]);

    assert_eq!(query.params.len(), 1);
    assert!(matches!(query.params[0], DataType::Int32(Some(1))));
}

#[test]
fn delete_all() {
    let query = DeleteBuilder::new("items").build().unwrap();

    assert_sql_contains(&query.sql, &["DELETE FROM items"]);
    assert_eq!(query.params.len(), 0);
}
