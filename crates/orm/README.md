# Omnia ORM

A lightweight object-relational mapper based on [`sea-query`](https://crates.io/crates/sea-query) but completely backend agnostic. This crate is intended as a helper for guests using `omnia-wasi-sql` to assist in query building and mapping return values to business structures.

This crate uses types from `omnia-wasi-sql` and re-exports `Row` and `DataType` for convenience.

It is intended that this crate is used in guest components only.

## Quick Start

### Define an Entity

```rust,ignore
use chrono::{DateTime, Utc};
use omnia_orm::entity; // Assuming re-exported or used directly

entity! {
    table = "posts",
    #[derive(Debug, Clone)]
    pub struct Post {
        pub id: i32,
        pub title: String,
        pub content: String,
        pub published: bool,
        pub created_at: DateTime<Utc>,
    }
}
```

### CRUD Operations

```rust,ignore
use omnia_wasi_sql::orm::{SelectBuilder, InsertBuilder, UpdateBuilder, DeleteBuilder, Filter};

// Select with filter
let posts = SelectBuilder::<Post>::new()
    .where(Filter::eq("published", true))
    .where(Filter::gt("created_at", Utc::now() - Duration::days(7)))
    .order_by_desc(None, "created_at")
    .limit(10)
    .fetch(provider, "db").await?;

// Insert
InsertBuilder::<Post>::new()
    .set("title", "Hello World")
    .set("content", "My first post")
    .set("published", true)
    .build()?;

// Or insert from an entity
let post = Post {
    id: 1,
    title: "Hello".to_string(),
    content: "World".to_string(),
    published: true,
    created_at: Utc::now(),
};
InsertBuilder::<Post>::from_entity(&post).build()?;

// Update
UpdateBuilder::<Post>::new()
    .set("published", true)
    .where(Filter::eq("id", 42))
    .build()?;

// Delete
DeleteBuilder::<Post>::new()
    .where(Filter::eq("id", 42))
    .build()?;
```

### Joins

```rust,ignore
use omnia_wasi_sql::orm::Join;

// Entity with default joins and column aliasing
entity! {
    table = "posts",
    columns = [
        ("users", "name", "author_name"),       // users.name AS author_name
        ("users", "email", "author_email"),    // users.email AS author_email
    ],
    joins = [
        Join::left("users", Filter::col_eq("posts", "author_id", "users", "id")),
    ],
    #[derive(Debug, Clone)]
    pub struct PostWithAuthor {
        pub id: i32,              // Auto: posts.id
        pub title: String,        // Auto: posts.title
        pub author_name: String,  // Manual: users.name AS author_name
        pub author_email: String, // Manual: users.email AS author_email
    }
}

// Joins happen automatically
let posts = SelectBuilder::<PostWithAuthor>::new()
    .fetch(provider, "db").await?;

// Or add ad-hoc joins
let posts = SelectBuilder::<Post>::new()
    .join(Join::left("users", Filter::col_eq("posts", "author_id", "users", "id")))
    .fetch(provider, "db").await?;
```

### Filtering

```rust,ignore
// Basic comparisons
Filter::eq("status", "active")
Filter::gt("views", 1000)
Filter::like("title", "%rust%")
Filter::in("id", vec![1, 2, 3])

// Logical combinators
Filter::and(vec![
    Filter::eq("published", true),
    Filter::gt("views", 100),
])

Filter::or(vec![
    Filter::eq("featured", true),
    Filter::gt("views", 5000),
])

// Table-qualified (for joins)
Filter::table_eq("posts", "published", true)
Filter::col_eq("posts", "author_id", "users", "id")
```

### Upserts

Handle `INSERT ... ON CONFLICT` scenarios using native database syntax (PostgreSQL/SQLite). These are **atomic operations**.

```rust,ignore
// Insert or update on conflict - atomic operation
InsertBuilder::<User>::new()
    .set("email", "test@example.com")
    .set("name", "John Doe")
    .on_conflict("email")
    .do_update(&["name"])
    .build()?;
// Generates: INSERT INTO users (email, name) VALUES ($1, $2)
//            ON CONFLICT (email) DO UPDATE SET name = EXCLUDED.name
```

## License

MIT OR Apache-2.0
