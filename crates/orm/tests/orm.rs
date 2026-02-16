//! Integration tests for ORM query builders.
//!
//! Tests the public API as users would interact with it.

#![cfg(target_arch = "wasm32")]
#![allow(missing_docs)]

mod common;

use common::{CommentWithAlias, Item, PostWithJoin, User, assert_sql_contains};
use qwasr_orm::{DeleteBuilder, Entity, Filter, InsertBuilder, Join, SelectBuilder, UpdateBuilder};
use qwasr_wasi_sql::types::DataType;

// SELECT tests

#[test]
fn select_basic() {
    let query = SelectBuilder::<User>::new().build().unwrap();
    assert_sql_contains(&query.sql, &["SELECT users.id, users.name, users.active", "FROM users"]);
    assert_eq!(query.params.len(), 0);
}

#[test]
fn select_with_ordering_and_limts() {
    let query = SelectBuilder::<User>::new()
        .r#where(Filter::eq("active", true))
        .r#where(Filter::gt("id", 100))
        .order_by(Some(User::TABLE), "id")
        .order_by_desc(None, "name")
        .limit(10)
        .offset(5)
        .build()
        .unwrap();

    assert_sql_contains(
        &query.sql,
        &[
            "SELECT users.id, users.name, users.active",
            "WHERE ((users.active) = ($1)) AND ((users.id) > ($2))",
            "ORDER BY users.id ASC, users.name DESC",
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
    let query = SelectBuilder::<CommentWithAlias>::new().build().unwrap();

    assert_sql_contains(
        &query.sql,
        &[
            "SELECT comments.id, comments.content, users.name AS author_name",
            "FROM comments",
            "LEFT JOIN users ON (comments.user_id) = (users.id)",
        ],
    );
}

#[test]
fn select_with_join() {
    let query = SelectBuilder::<PostWithJoin>::new().build().unwrap();

    assert_sql_contains(
        &query.sql,
        &[
            "SELECT posts.id, posts.title",
            "FROM posts",
            "LEFT JOIN users ON (posts.author_id) = (users.id)",
        ],
    );
}

#[test]
fn select_with_ad_hoc_join() {
    let query = SelectBuilder::<User>::new()
        .join(Join::inner("user_roles", Filter::col_eq("users", "id", "user_roles", "user_id")))
        .build()
        .unwrap();

    assert_sql_contains(
        &query.sql,
        &[
            "SELECT users.id, users.name, users.active",
            "INNER JOIN user_roles ON (users.id) = (user_roles.user_id)",
        ],
    );
}

#[test]
fn select_with_or_filter() {
    let query = SelectBuilder::<User>::new()
        .r#where(Filter::Or(vec![Filter::eq("active", true), Filter::gt("id", 100)]))
        .build()
        .unwrap();

    assert_sql_contains(
        &query.sql,
        &[
            "SELECT users.id, users.name, users.active",
            "WHERE ((users.active) = ($1)) OR ((users.id) > ($2))",
        ],
    );
}

#[test]
fn select_with_not_filter() {
    let query = SelectBuilder::<User>::new()
        .r#where(Filter::Not(Box::new(Filter::eq("active", false))))
        .build()
        .unwrap();

    assert_sql_contains(
        &query.sql,
        &["SELECT users.id, users.name, users.active", "WHERE NOT ((users.active) = ($1))"],
    );
}

#[test]
fn select_with_right_join() {
    let query = SelectBuilder::<User>::new()
        .join(Join::right("profiles", Filter::col_eq("users", "id", "profiles", "user_id")))
        .build()
        .unwrap();

    assert_sql_contains(
        &query.sql,
        &["SELECT users.id, users.name, users.active", "FROM users", "RIGHT JOIN profiles"],
    );
}

#[test]
fn select_with_full_join() {
    let query = SelectBuilder::<User>::new()
        .join(Join::full("accounts", Filter::col_eq("users", "id", "accounts", "user_id")))
        .build()
        .unwrap();

    assert_sql_contains(
        &query.sql,
        &["SELECT users.id, users.name, users.active", "FROM users", "FULL OUTER JOIN accounts"],
    );
}

#[test]
fn select_with_multiple_join_types() {
    let query = SelectBuilder::<User>::new()
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
    let query = InsertBuilder::<Item>::new().set("name", "test").set("count", 42).build().unwrap();

    assert_sql_contains(&query.sql, &["INSERT INTO items (name, count) VALUES ($1, $2)"]);

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

    let query = InsertBuilder::<Item>::from_entity(&item).build().unwrap();

    assert_sql_contains(&query.sql, &["INSERT INTO items (id, name, count) VALUES ($1, $2, $3)"]);

    assert_eq!(query.params.len(), 3);
    assert!(matches!(query.params[0], DataType::Int64(Some(1))));
    assert!(matches!(&query.params[1], DataType::Str(Some(s)) if s == "test"));
    assert!(matches!(query.params[2], DataType::Int32(Some(10))));
}

#[test]
fn insert_with_upsert() {
    let query = InsertBuilder::<Item>::new()
        .set("name", "unique")
        .set("count", 1)
        .on_conflict("name")
        .do_update(&["count"])
        .build()
        .unwrap();

    assert_sql_contains(
        &query.sql,
        &[
            "INSERT INTO items (name, count) VALUES ($1, $2)",
            "ON CONFLICT (name)",
            "DO UPDATE",
            "SET count = excluded.count",
        ],
    );

    assert_eq!(query.params.len(), 2);
    assert!(matches!(&query.params[0], DataType::Str(Some(s)) if s == "unique"));
    assert!(matches!(query.params[1], DataType::Int32(Some(1))));
}

#[test]
fn insert_upsert_do_nothing() {
    let query = InsertBuilder::<Item>::new()
        .set("name", "test")
        .on_conflict("name")
        .do_nothing()
        .build()
        .unwrap();

    assert_sql_contains(
        &query.sql,
        &["INSERT INTO items (name) VALUES ($1)", "ON CONFLICT (name) DO NOTHING"],
    );

    assert_eq!(query.params.len(), 1);
    assert!(matches!(&query.params[0], DataType::Str(Some(s)) if s == "test"));
}

// UPDATE tests

#[test]
fn update_basic() {
    let query = UpdateBuilder::<Item>::new()
        .set("name", "updated")
        .r#where(Filter::eq("id", 1))
        .build()
        .unwrap();

    assert_sql_contains(&query.sql, &["UPDATE items", "SET name = $1", "WHERE (items.id) = ($2)"]);

    assert_eq!(query.params.len(), 2);
    assert!(matches!(&query.params[0], DataType::Str(Some(s)) if s == "updated"));
    assert!(matches!(query.params[1], DataType::Int32(Some(1))));
}

#[test]
fn update_multiple_fields() {
    let query = UpdateBuilder::<Item>::new()
        .set("name", "new")
        .set("id", 99)
        .r#where(Filter::eq("id", 1))
        .build()
        .unwrap();

    assert_sql_contains(
        &query.sql,
        &["UPDATE items", "SET name = $1, id = $2", "WHERE (items.id) = ($3)"],
    );

    assert_eq!(query.params.len(), 3);
    assert!(matches!(&query.params[0], DataType::Str(Some(s)) if s == "new"));
    assert!(matches!(query.params[1], DataType::Int32(Some(99))));
    assert!(matches!(query.params[2], DataType::Int32(Some(1))));
}

#[test]
fn update_no_filter() {
    let query = UpdateBuilder::<Item>::new().set("name", "global").build().unwrap();

    assert_sql_contains(&query.sql, &["UPDATE items", "SET name = $1"]);

    assert_eq!(query.params.len(), 1);
    assert!(matches!(&query.params[0], DataType::Str(Some(s)) if s == "global"));
}

#[test]
fn update_with_returning() {
    let query = UpdateBuilder::<Item>::new()
        .set("name", "updated")
        .r#where(Filter::eq("id", 1))
        .returning("name")
        .build()
        .unwrap();

    assert_sql_contains(
        &query.sql,
        &["UPDATE items", "SET name = $1", "WHERE (items.id) = ($2)", "RETURNING name"],
    );
    assert_eq!(query.params.len(), 2);
}

#[test]
fn update_with_multiple_returning() {
    let query = UpdateBuilder::<Item>::new()
        .set("name", "updated")
        .r#where(Filter::eq("id", 1))
        .returning("id")
        .returning("name")
        .build()
        .unwrap();

    // ``SeaQuery`` picks the last one
    assert_sql_contains(&query.sql, &["UPDATE items", "SET name = $1", "RETURNING name"]);
}

// DELETE tests

#[test]
fn delete_with_filter() {
    let query = DeleteBuilder::<Item>::new().r#where(Filter::eq("id", 1)).build().unwrap();

    assert_sql_contains(&query.sql, &["DELETE FROM items", "WHERE (items.id) = ($1)"]);

    assert_eq!(query.params.len(), 1);
    assert!(matches!(query.params[0], DataType::Int32(Some(1))));
}

#[test]
fn delete_all() {
    let query = DeleteBuilder::<Item>::new().build().unwrap();

    assert_sql_contains(&query.sql, &["DELETE FROM items"]);
    assert_eq!(query.params.len(), 0);
}

#[test]
fn delete_with_returning() {
    let query = DeleteBuilder::<Item>::new()
        .r#where(Filter::eq("id", 1))
        .returning("name")
        .build()
        .unwrap();

    assert_sql_contains(
        &query.sql,
        &["DELETE FROM items", "WHERE (items.id) = ($1)", "RETURNING name"],
    );
    assert_eq!(query.params.len(), 1);
    assert!(matches!(query.params[0], DataType::Int32(Some(1))));
}
