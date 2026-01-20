//! Integration tests for ORM query builders.
//!
//! Tests the public API as users would interact with it.

#![cfg(target_arch = "wasm32")]
#![allow(missing_docs)]
use qwasr_wasi_sql::entity;
use qwasr_wasi_sql::orm::{
    DeleteBuilder, Filter, InsertBuilder, Join, SelectBuilder, UpdateBuilder,
};
use qwasr_wasi_sql::types::DataType;

// Test entities
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
    table = "records",
    #[derive(Debug, Clone)]
    pub struct Record {
        pub id: i64,
        pub value: String,
    }
}

// Helpers below intentionally normalize and fragment-check SQL so we assert ORM intent
// without snapshotting SeaQuery's exact formatting choices.
fn normalize_sql(sql: &str) -> String {
    sql.split_whitespace().collect::<Vec<_>>().join(" ")
}

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

fn assert_sql_contains(actual: &str, fragments: &[&str]) {
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

// SELECT tests

#[test]
fn select_basic() {
    let query = SelectBuilder::<User>::new().build().unwrap();

    assert_sql_contains(&query.sql, &["SELECT users.id, users.name, users.active", "FROM users"]);

    assert_eq!(query.params.len(), 0);
}

#[test]
fn select_with_filter() {
    let query = SelectBuilder::<User>::new().r#where(Filter::eq("active", true)).build().unwrap();

    assert_sql_contains(
        &query.sql,
        &["SELECT users.id, users.name, users.active", "FROM users", "WHERE (users.active) = ($1)"],
    );

    assert_eq!(query.params.len(), 1);
    assert!(matches!(query.params[0], DataType::Boolean(Some(true))));
}

#[test]
fn select_with_limit_offset() {
    let query = SelectBuilder::<User>::new().limit(10).offset(5).build().unwrap();

    assert_sql_contains(
        &query.sql,
        &["SELECT users.id, users.name, users.active", "FROM users", "LIMIT $1", "OFFSET $2"],
    );

    assert_eq!(query.params.len(), 2);
    assert!(matches!(query.params[0], DataType::Uint64(Some(10))));
    assert!(matches!(query.params[1], DataType::Uint64(Some(5))));
}

#[test]
fn select_with_ordering() {
    let query = SelectBuilder::<User>::new().order_by(None, "name").build().unwrap();

    assert_sql_contains(
        &query.sql,
        &["SELECT users.id, users.name, users.active", "ORDER BY users.name ASC"],
    );

    let query_desc = SelectBuilder::<User>::new().order_by_desc(None, "id").build().unwrap();

    assert_sql_contains(
        &query_desc.sql,
        &["SELECT users.id, users.name, users.active", "ORDER BY users.id DESC"],
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
fn select_multiple_filters() {
    let query = SelectBuilder::<User>::new()
        .r#where(Filter::eq("active", true))
        .r#where(Filter::gt("id", 10))
        .r#where(Filter::lte("id", 100))
        .build()
        .unwrap();

    assert_sql_contains(
        &query.sql,
        &[
            "SELECT users.id, users.name, users.active",
            "WHERE ((users.active) = ($1)) AND ((users.id) > ($2)) AND ((users.id) <= ($3))",
        ],
    );

    assert_eq!(query.params.len(), 3);
    assert!(matches!(query.params[0], DataType::Boolean(Some(true))));
    assert!(matches!(query.params[1], DataType::Int32(Some(10))));
    assert!(matches!(query.params[2], DataType::Int32(Some(100))));
}

#[test]
fn select_no_filters() {
    let query = SelectBuilder::<User>::new().build().unwrap();

    assert_sql_contains(&query.sql, &["SELECT users.id, users.name, users.active", "FROM users"]);
    assert_eq!(query.params.len(), 0);
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
fn select_with_table_qualified_ordering() {
    let query = SelectBuilder::<User>::new().order_by(Some("users"), "name").build().unwrap();

    assert_sql_contains(
        &query.sql,
        &["SELECT users.id, users.name, users.active", "ORDER BY users.name ASC"],
    );
}

#[test]
fn select_combined_query() {
    let query = SelectBuilder::<User>::new()
        .r#where(Filter::eq("active", true))
        .r#where(Filter::gt("id", 100))
        .order_by(None, "name")
        .limit(10)
        .offset(5)
        .build()
        .unwrap();

    assert_sql_contains(
        &query.sql,
        &[
            "SELECT users.id, users.name, users.active",
            "WHERE ((users.active) = ($1)) AND ((users.id) > ($2))",
            "ORDER BY users.name ASC",
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
fn select_multiple_orderings() {
    let query = SelectBuilder::<User>::new()
        .order_by(None, "active")
        .order_by_desc(None, "name")
        .order_by(None, "id")
        .build()
        .unwrap();

    assert_sql_contains(&query.sql, &["ORDER BY users.active ASC, users.name DESC, users.id ASC"]);
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
fn select_full_sql() {
    let query = SelectBuilder::<User>::new()
        .r#where(Filter::eq("active", true))
        .order_by(None, "name")
        .limit(5)
        .offset(10)
        .build()
        .unwrap();

    assert_sql_contains(
        &query.sql,
        &[
            "SELECT users.id, users.name, users.active",
            "WHERE (users.active) = ($1)",
            "ORDER BY users.name ASC",
            "LIMIT $2",
            "OFFSET $3",
        ],
    );

    assert!(matches!(query.params[0], DataType::Boolean(Some(true))));
    assert!(matches!(query.params[1], DataType::Uint64(Some(5))));
    assert!(matches!(query.params[2], DataType::Uint64(Some(10))));
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
    let query = UpdateBuilder::<Record>::new()
        .set("value", "updated")
        .r#where(Filter::eq("id", 1))
        .build()
        .unwrap();

    assert_sql_contains(
        &query.sql,
        &["UPDATE records", "SET value = $1", "WHERE (records.id) = ($2)"],
    );

    assert_eq!(query.params.len(), 2);
    assert!(matches!(&query.params[0], DataType::Str(Some(s)) if s == "updated"));
    assert!(matches!(query.params[1], DataType::Int32(Some(1))));
}

#[test]
fn update_multiple_fields() {
    let query = UpdateBuilder::<Record>::new()
        .set("value", "new")
        .set("id", 99)
        .r#where(Filter::eq("id", 1))
        .build()
        .unwrap();

    assert_sql_contains(
        &query.sql,
        &["UPDATE records", "SET value = $1, id = $2", "WHERE (records.id) = ($3)"],
    );

    assert_eq!(query.params.len(), 3);
    assert!(matches!(&query.params[0], DataType::Str(Some(s)) if s == "new"));
    assert!(matches!(query.params[1], DataType::Int32(Some(99))));
    assert!(matches!(query.params[2], DataType::Int32(Some(1))));
}

#[test]
fn update_no_filter() {
    let query = UpdateBuilder::<Record>::new().set("value", "global").build().unwrap();

    assert_sql_contains(&query.sql, &["UPDATE records", "SET value = $1"]);

    assert_eq!(query.params.len(), 1);
    assert!(matches!(&query.params[0], DataType::Str(Some(s)) if s == "global"));
}

// DELETE tests

#[test]
fn delete_with_filter() {
    let query = DeleteBuilder::<Record>::new().r#where(Filter::eq("id", 1)).build().unwrap();

    assert_sql_contains(&query.sql, &["DELETE FROM records", "WHERE (records.id) = ($1)"]);

    assert_eq!(query.params.len(), 1);
    assert!(matches!(query.params[0], DataType::Int32(Some(1))));
}

#[test]
fn delete_all() {
    let query = DeleteBuilder::<Record>::new().build().unwrap();

    assert_sql_contains(&query.sql, &["DELETE FROM records"]);
    assert_eq!(query.params.len(), 0);
}

#[test]
fn delete_with_returning() {
    let query = DeleteBuilder::<Record>::new()
        .r#where(Filter::eq("id", 1))
        .returning("value")
        .build()
        .unwrap();

    assert_sql_contains(
        &query.sql,
        &["DELETE FROM records", "WHERE (records.id) = ($1)", "RETURNING value"],
    );
    assert_eq!(query.params.len(), 1);
    assert!(matches!(query.params[0], DataType::Int32(Some(1))));
}

// ENTITY MACRO tests

#[test]
fn entity_macro_basic() {
    use qwasr_wasi_sql::orm::Entity;

    entity! {
        table = "test_users",
        pub struct TestUser {
            pub id: i64,
            pub name: String,
            pub active: bool,
        }
    }

    assert_eq!(TestUser::TABLE, "test_users");
    assert_eq!(TestUser::projection(), &["id", "name", "active"]);
    assert!(TestUser::joins().is_empty());
    assert!(TestUser::column_specs().is_empty());
    assert!(TestUser::ordering().is_empty());
}

#[test]
fn entity_macro_from_row() {
    use qwasr_wasi_sql::orm::Entity;
    use qwasr_wasi_sql::types::{Field, Row};

    entity! {
        table = "test_posts",
        pub struct TestPost {
            pub id: i64,
            pub title: String,
            pub published: bool,
        }
    }

    let row = Row {
        fields: vec![
            Field {
                name: "id".to_string(),
                value: DataType::Int64(Some(1)),
            },
            Field {
                name: "title".to_string(),
                value: DataType::Str(Some("Hello World".to_string())),
            },
            Field {
                name: "published".to_string(),
                value: DataType::Boolean(Some(true)),
            },
        ],
        index: "0".to_string(),
    };

    let post = TestPost::from_row(&row).unwrap();
    assert_eq!(post.id, 1);
    assert_eq!(post.title, "Hello World");
    assert!(post.published);
}

#[test]
fn entity_macro_with_optional_fields() {
    use qwasr_wasi_sql::orm::Entity;
    use qwasr_wasi_sql::types::{Field, Row};

    entity! {
        table = "test_articles",
        pub struct TestArticle {
            pub id: i64,
            pub title: String,
            pub subtitle: Option<String>,
        }
    }

    let row = Row {
        fields: vec![
            Field {
                name: "id".to_string(),
                value: DataType::Int64(Some(42)),
            },
            Field {
                name: "title".to_string(),
                value: DataType::Str(Some("Main Title".to_string())),
            },
            Field {
                name: "subtitle".to_string(),
                value: DataType::Str(None),
            },
        ],
        index: "0".to_string(),
    };

    let article = TestArticle::from_row(&row).unwrap();
    assert_eq!(article.id, 42);
    assert_eq!(article.title, "Main Title");
    assert_eq!(article.subtitle, None);
}

#[test]
fn entity_macro_with_joins() {
    use qwasr_wasi_sql::orm::Entity;

    entity! {
        table = "test_comments",
        joins = [
            Join::inner("users", Filter::col_eq("test_comments", "user_id", "users", "id"))
        ],
        pub struct TestCommentWithUser {
            pub id: i64,
            pub content: String,
            pub user_name: String,
        }
    }

    assert_eq!(TestCommentWithUser::TABLE, "test_comments");
    assert_eq!(TestCommentWithUser::projection(), &["id", "content", "user_name"]);

    let joins = TestCommentWithUser::joins();
    assert_eq!(joins.len(), 1);
}

#[test]
fn entity_macro_with_joins_and_columns() {
    use qwasr_wasi_sql::orm::Entity;

    entity! {
        table = "test_orders",
        columns = [
            ("test_orders", "id", "id"),
            ("test_orders", "total", "total"),
            ("customers", "name", "customer_name"),
        ],
        joins = [
            Join::left("customers", Filter::col_eq("test_orders", "customer_id", "customers", "id"))
        ],
        pub struct TestOrderWithCustomer {
            pub id: i64,
            pub total: f64,
            pub customer_name: String,
        }
    }

    assert_eq!(TestOrderWithCustomer::TABLE, "test_orders");
    assert_eq!(TestOrderWithCustomer::projection(), &["id", "total", "customer_name"]);

    let column_specs = TestOrderWithCustomer::column_specs();
    assert_eq!(column_specs.len(), 3);
    assert_eq!(column_specs[0], ("id", "test_orders", "id"));
    assert_eq!(column_specs[1], ("total", "test_orders", "total"));
    assert_eq!(column_specs[2], ("customer_name", "customers", "name"));

    let joins = TestOrderWithCustomer::joins();
    assert_eq!(joins.len(), 1);
}

#[test]
fn entity_values_trait() {
    use qwasr_wasi_sql::orm::EntityValues;

    entity! {
        table = "test_products",
        pub struct TestProduct {
            pub id: i64,
            pub name: String,
            pub price: f64,
        }
    }

    let product = TestProduct {
        id: 100,
        name: "Widget".to_string(),
        price: 29.99,
    };

    let values = product.__to_values();
    assert_eq!(values.len(), 3);
    assert_eq!(values[0].0, "id");
    assert_eq!(values[1].0, "name");
    assert_eq!(values[2].0, "price");
}

#[test]
fn entity_from_row_missing_field() {
    use qwasr_wasi_sql::orm::Entity;
    use qwasr_wasi_sql::types::{Field, Row};

    entity! {
        table = "test_items",
        pub struct TestItemMissing {
            pub id: i64,
            pub name: String,
        }
    }

    let row = Row {
        fields: vec![Field {
            name: "id".to_string(),
            value: DataType::Int64(Some(1)),
        }],
        index: "0".to_string(),
    };

    let result = TestItemMissing::from_row(&row);
    assert!(result.is_err());
    let err = result.err().unwrap();
    assert!(err.to_string().contains("missing column"));
}

#[test]
fn entity_from_row_wrong_type() {
    use qwasr_wasi_sql::orm::Entity;
    use qwasr_wasi_sql::types::{Field, Row};

    entity! {
        table = "test_records",
        pub struct TestRecordWrong {
            pub id: i64,
            pub count: i32,
        }
    }

    let row = Row {
        fields: vec![
            Field {
                name: "id".to_string(),
                value: DataType::Int64(Some(1)),
            },
            Field {
                name: "count".to_string(),
                value: DataType::Str(Some("not_a_number".to_string())),
            },
        ],
        index: "0".to_string(),
    };

    let result = TestRecordWrong::from_row(&row);
    assert!(result.is_err());
}

#[test]
#[allow(clippy::struct_field_names, clippy::approx_constant, clippy::float_cmp)]
fn entity_with_multiple_types() {
    use qwasr_wasi_sql::orm::Entity;
    use qwasr_wasi_sql::types::{Field, Row};

    entity! {
        table = "test_types",
        pub struct TestAllTypes {
            pub bool_field: bool,
            pub i32_field: i32,
            pub i64_field: i64,
            pub u32_field: u32,
            pub u64_field: u64,
            pub f32_field: f32,
            pub f64_field: f64,
            pub string_field: String,
            pub bytes_field: Vec<u8>,
        }
    }

    let row = Row {
        fields: vec![
            Field {
                name: "bool_field".to_string(),
                value: DataType::Boolean(Some(true)),
            },
            Field {
                name: "i32_field".to_string(),
                value: DataType::Int32(Some(42)),
            },
            Field {
                name: "i64_field".to_string(),
                value: DataType::Int64(Some(1000)),
            },
            Field {
                name: "u32_field".to_string(),
                value: DataType::Uint32(Some(100)),
            },
            Field {
                name: "u64_field".to_string(),
                value: DataType::Uint64(Some(2000)),
            },
            Field {
                name: "f32_field".to_string(),
                value: DataType::Float(Some(3.14)),
            },
            Field {
                name: "f64_field".to_string(),
                value: DataType::Double(Some(2.718)),
            },
            Field {
                name: "string_field".to_string(),
                value: DataType::Str(Some("test".to_string())),
            },
            Field {
                name: "bytes_field".to_string(),
                value: DataType::Binary(Some(vec![1, 2, 3])),
            },
        ],
        index: "0".to_string(),
    };

    let result = TestAllTypes::from_row(&row).unwrap();
    assert!(result.bool_field);
    assert_eq!(result.i32_field, 42);
    assert_eq!(result.i64_field, 1000);
    assert_eq!(result.u32_field, 100);
    assert_eq!(result.u64_field, 2000);
    assert_eq!(result.f32_field, 3.14);
    assert_eq!(result.f64_field, 2.718);
    assert_eq!(result.string_field, "test");
    assert_eq!(result.bytes_field, vec![1, 2, 3]);
}
