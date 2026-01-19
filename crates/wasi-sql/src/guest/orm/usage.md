# ORM Usage Guide

Comprehensive guide to using the ORM layer with examples covering implemented features.

## Table of Contents

1. [Entity Definition](#entity-definition)
2. [Basic CRUD Operations](#basic-crud-operations)
3. [Filtering](#filtering)
4. [Joins](#joins)
5. [Upserts](#upserts)
6. [Ordering & Pagination](#ordering--pagination)
7. [Custom Types](#custom-types)

---

## Entity Definition

### Basic Entity

```rust
use crate::orm::{Entity, FetchValue};
use chrono::{DateTime, Utc};

entity! {
    table = "posts",
    #[derive(Debug, Clone)]
    pub struct Post {
        pub id: i32,
        pub title: String,
        pub content: String,
        pub author_id: i32,
        pub created_at: DateTime<Utc>,
        pub published: bool,
    }
}
```

### Entity with Default Joins

```rust
use crate::orm::Join;

entity! {
    table = "posts",
    joins = [
        Join::left("users", Filter::col_eq("posts", "author_id", "users", "id")),
    ],
    #[derive(Debug, Clone)]
    pub struct PostWithAuthor {
        pub id: i32,
        pub title: String,
        pub author_id: i32,
        pub author_name: String,     // From joined users table
        pub author_email: String,    // From joined users table
    }
}
```

### Entity with Column Aliasing

When joining tables, use the `columns` parameter to explicitly specify which fields come from joined tables:

```rust
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
        pub id: i32,              // Auto-qualified: posts.id
        pub title: String,        // Auto-qualified: posts.title
        pub author_id: i32,       // Auto-qualified: posts.author_id
        pub author_name: String,  // Manual: users.name AS author_name
        pub author_email: String, // Manual: users.email AS author_email
    }
}
```

**Key Points:**

- Fields **not** in `columns` are auto-qualified with the main table (`posts.id`, `posts.title`, etc.)
- Fields **in** `columns` use explicit table qualification: `(source_table, source_column, struct_field)`
- This is required when selecting columns from joined tables
- Useful for resolving name conflicts (e.g., both tables have `created_at`)

**Handling Column Name Conflicts:**

```rust
entity! {
    table = "posts",
    columns = [
        ("users", "created_at", "author_created_at"),  // Alias to avoid conflict
    ],
    joins = [
        Join::left("users", Filter::col_eq("posts", "author_id", "users", "id")),
    ],
    #[derive(Debug, Clone)]
    pub struct PostFull {
        pub id: i32,
        pub title: String,
        pub created_at: DateTime<Utc>,        // posts.created_at
        pub author_created_at: DateTime<Utc>, // users.created_at AS author_created_at
    }
}
```

---

## Basic CRUD Operations

### Select All

```rust
use crate::orm::SelectBuilder;

let posts = SelectBuilder::<Post>::new()
    .fetch(provider, "db").await?;

for post in posts {
    println!("{}: {}", post.id, post.title);
}
```

### Select with Filter

```rust
let published_posts = SelectBuilder::<Post>::new()
    .where(Filter::eq("published", true))
    .fetch(provider, "db").await?;
```

### Insert

```rust
use crate::orm::InsertBuilder;

// Basic insert
InsertBuilder::<Post>::new()
    .set("title", "Hello World")
    .set("content", "This is my first post")
    .set("author_id", 42)
    .set("published", true)
    .build()?;

// Insert from an entity instance
let post = Post {
    id: 1,
    title: "Hello World".to_string(),
    content: "This is my first post".to_string(),
    author_id: 42,
    published: true,
    created_at: Utc::now(),
};
InsertBuilder::<Post>::from_entity(&post).build()?;
```

### Update

```rust
use crate::orm::UpdateBuilder;

UpdateBuilder::<Post>::new()
    .set("title", "Updated Title")
    .set("published", true)
    .where(Filter::eq("id", 123))
    .build()?;
```

### Delete

```rust
use crate::orm::DeleteBuilder;

DeleteBuilder::<Post>::new()
    .where(Filter::eq("id", 123))
    .build()?;
```

---

## Filtering

### Basic Comparisons

```rust
// Equality
Filter::eq("status", "active")
Filter::ne("status", "deleted")

// Numeric comparisons
Filter::gt("views", 1000)
Filter::gte("views", 100)
Filter::lt("age", 18)
Filter::lte("price", 99.99)

// String patterns
Filter::like("title", "%rust%")
Filter::not_like("content", "%spam%")

// Null checks
Filter::is_null("deleted_at")
Filter::is_not_null("published_at")

// Range
Filter::between("price", 10.0, 50.0)
Filter::not_between("age", 13, 19)
```

### Collections

```rust
// IN clause
Filter::in("status", vec!["active", "pending", "approved"])
Filter::not_in("user_id", vec![1, 2, 3])

// ANY clause (for array columns)
Filter::any("tags", vec!["rust", "programming"])
```

### Table-Qualified Filters (for Joins)

```rust
// Useful when you have joins
Filter::table_eq("posts", "published", true)
Filter::table_gt("users", "age", 18)
Filter::table_like("posts", "title", "%rust%")
```

### Column-to-Column Comparisons

```rust
// Compare columns from different tables
Filter::col_eq("posts", "author_id", "users", "id")
Filter::col_gt("orders", "total", "users", "credit_limit")
```

### Logical Combinators

```rust
// AND
Filter::and(vec![
    Filter::eq("published", true),
    Filter::gt("views", 100),
])

// OR
Filter::or(vec![
    Filter::eq("status", "featured"),
    Filter::gt("views", 10000),
])

// NOT
Filter::not(Filter::eq("deleted", true))

// Complex combinations
Filter::and(vec![
    Filter::eq("published", true),
    Filter::or(vec![
        Filter::eq("featured", true),
        Filter::gt("views", 5000),
    ]),
])
```

### Chaining Filters in SelectBuilder

```rust
SelectBuilder::<Post>::new()
    .where(Filter::eq("published", true))
    .where(Filter::gt("created_at", Utc::now() - Duration::days(7)))
    .where(Filter::like("title", "%rust%"))
    .fetch(provider, "db").await?;
```

---

## Joins

### Entity-Level Default Joins

Joins defined in the entity macro are applied automatically:

```rust
entity! {
    table = "posts",
    joins = [
        Join::left("users", Filter::col_eq("posts", "author_id", "users", "id")),
        Join::left("categories", Filter::col_eq("posts", "category_id", "categories", "id")),
    ],
    #[derive(Debug, Clone)]
    pub struct PostFull {
        pub id: i32,
        pub title: String,
        pub author_name: String,
        pub category_name: String,
    }
}

// Joins happen automatically
let posts = SelectBuilder::<PostFull>::new()
    .fetch(provider, "db").await?;
```

### Ad-Hoc Query Joins

Override or add joins for specific queries:

```rust
// Simple entity without default joins
let posts = SelectBuilder::<Post>::new()
    .join(Join::left("users", Filter::col_eq("posts", "author_id", "users", "id")))
    .join(Join::inner("categories", Filter::col_eq("posts", "category_id", "categories", "id")))
    .fetch(provider, "db").await?;
```

### Join Types

```rust
// Inner join (default)
Join::new("users", Filter::col_eq("posts", "author_id", "users", "id"))
Join::inner("users", Filter::col_eq("posts", "author_id", "users", "id"))  // Alias

// Left join
Join::left("users", Filter::col_eq("posts", "author_id", "users", "id"))

// Right join
Join::right("users", Filter::col_eq("posts", "author_id", "users", "id"))

// Full outer join
Join::full("users", Filter::col_eq("posts", "author_id", "users", "id"))
```

### Table Aliases

```rust
Join::left("users", Filter::col_eq("posts", "author_id", "users", "id"))
    .alias("author")
```

### Complex Join Conditions

```rust
Join::left("users", Filter::and(vec![
    Filter::col_eq("posts", "author_id", "users", "id"),
    Filter::table_eq("users", "active", true),
]))
```

---

## Upserts

Handle INSERT ... ON CONFLICT scenarios using native database syntax.

**Important:** Upserts are **atomic database operations**. The ORM generates a single `INSERT ... ON CONFLICT` SQL statement that the database executes atomically. This is **not** a "select first, then insert or update" pattern - it's faster, safer, and avoids race conditions.

**Generated SQL Example:**

```sql
-- What .on_conflict("email").do_update(&["name"]) generates:
INSERT INTO users (email, name) VALUES ($1, $2)
ON CONFLICT (email) DO UPDATE SET name = EXCLUDED.name
```

### Do Nothing on Conflict

```rust
// Ignore insert if email already exists
InsertBuilder::<User>::new()
    .set("email", "test@example.com")
    .set("name", "John Doe")
    .on_conflict("email")
    .do_nothing()
    .build()?;
```

### Update on Conflict

```rust
// Update name if email already exists
InsertBuilder::<User>::new()
    .set("email", "test@example.com")
    .set("name", "John Updated")
    .set("age", 30)
    .on_conflict("email")
    .do_update(&["name", "age"])  // Update these columns
    .build()?;
```

### Multiple Column Conflicts

```rust
InsertBuilder::<User>::new()
    .set("email", "test@example.com")
    .set("username", "john")
    .set("name", "John")
    .on_conflict_columns(&["email", "username"])
    .do_update(&["name"])
    .build()?;
```

### Update All Except Conflict Columns

```rust
InsertBuilder::<User>::new()
    .set("email", "test@example.com")
    .set("username", "john")
    .set("name", "John")
    .set("age", 30)
    .set("bio", "Developer")
    .on_conflict_columns(&["email", "username"])
    .do_update_all()  // Updates name, age, bio (not email, username)
    .build()?;
```

---

## Ordering & Pagination

### Basic Ordering

```rust
// Ascending (default)
SelectBuilder::<Post>::new()
    .order_by(None, "created_at")
    .fetch(provider, "db").await?;

// Descending
SelectBuilder::<Post>::new()
    .order_by_desc(None, "created_at")
    .fetch(provider, "db").await?;
```

### Multiple Sort Columns

```rust
SelectBuilder::<Post>::new()
    .order_by_desc(None, "featured")  // Featured first
    .order_by_desc(None, "created_at")  // Then newest
    .fetch(provider, "db").await?;
```

### Ordering with Joins

```rust
SelectBuilder::<PostWithAuthor>::new()
    .order_by(Some("users"), "name")  // Order by author name
    .order_by_desc(Some("posts"), "created_at")
    .fetch(provider, "db").await?;
```

### Pagination

```rust
// Page 1: Items 1-10
SelectBuilder::<Post>::new()
    .where(Filter::eq("published", true))
    .order_by_desc(None, "created_at")
    .limit(10)
    .offset(0)
    .fetch(provider, "db").await?;

// Page 2: Items 11-20
SelectBuilder::<Post>::new()
    .where(Filter::eq("published", true))
    .order_by_desc(None, "created_at")
    .limit(10)
    .offset(10)
    .fetch(provider, "db").await?;
```

---

## Custom Types

### Implementing FetchValue for Custom Types

```rust
use crate::orm::FetchValue;
use wasi_sql::types::{DataType, Row};

// Custom newtype
pub struct UserId(String);

impl FetchValue for UserId {
    fn fetch(row: &Row, col: &str) -> anyhow::Result<Self> {
        let id: String = FetchValue::fetch(row, col)?;
        Ok(UserId(id))
    }
}

// Custom enum
pub enum UserStatus {
    Active,
    Suspended,
    Deleted,
}

impl FetchValue for UserStatus {
    fn fetch(row: &Row, col: &str) -> anyhow::Result<Self> {
        let status: String = FetchValue::fetch(row, col)?;
        match status.as_str() {
            "active" => Ok(UserStatus::Active),
            "suspended" => Ok(UserStatus::Suspended),
            "deleted" => Ok(UserStatus::Deleted),
            _ => Err(anyhow::anyhow!("Invalid user status: {}", status)),
        }
    }
}

// Use in entity
entity! {
    table = "users",
    #[derive(Debug)]
    pub struct User {
        pub id: UserId,
        pub status: UserStatus,
        pub name: String,
    }
}
```

### Supported Built-in Types

The ORM automatically handles these types via `FetchValue`:

- **Primitives:** `bool`, `i32`, `i64`, `u32`, `u64`, `f32`, `f64`
- **Collections:** `String`, `Vec<u8>`
- **Date/Time:** `DateTime<Utc>` (from `chrono`)
- **JSON:** `serde_json::Value`
- **Nullable:** `Option<T>` for any `FetchValue` type

### Using Natural Rust Types in Filters

```rust
use chrono::Utc;

// DateTime
let recent = SelectBuilder::<Post>::new()
    .where(Filter::gt("created_at", Utc::now() - Duration::days(7)))
    .fetch(provider, "db").await?;

// Integers
SelectBuilder::<Post>::new()
    .where(Filter::in("status_code", vec![200, 201, 204]))
    .fetch(provider, "db").await?;

// Booleans
SelectBuilder::<Post>::new()
    .where(Filter::eq("published", true))
    .fetch(provider, "db").await?;

// Strings
SelectBuilder::<Post>::new()
    .where(Filter::like("title", "%rust%"))
    .fetch(provider, "db").await?;
```

---

## Complete Examples

### Blog Post Listing with Author

```rust
entity! {
    table = "posts",
    columns = [
        ("users", "name", "author_name"),  // users.name AS author_name
    ],
    joins = [
        Join::left("users", Filter::col_eq("posts", "author_id", "users", "id")),
    ],
    #[derive(Debug, Clone)]
    pub struct PostListItem {
        pub id: i32,              // posts.id
        pub title: String,        // posts.title
        pub excerpt: String,      // posts.excerpt
        pub author_name: String,  // users.name AS author_name
        pub created_at: DateTime<Utc>,  // posts.created_at
        pub view_count: i32,      // posts.view_count
    }
}

// Get recent published posts
let posts = SelectBuilder::<PostListItem>::new()
    .where(Filter::eq("published", true))
    .where(Filter::gt("created_at", Utc::now() - Duration::days(30)))
    .order_by_desc(None, "created_at")
    .limit(20)
    .fetch(provider, "db").await?;
```

### User Registration with Upsert

```rust
// Register or update user info
InsertBuilder::<User>::new()
    .set("email", user_email)
    .set("username", username)
    .set("name", name)
    .set("created_at", Utc::now())
    .on_conflict("email")
    .do_update(&["username", "name"])
    .build()?;
```

### Complex Filtering

```rust
// Featured posts OR recent popular posts by active authors
SelectBuilder::<PostWithAuthor>::new()
    .where(Filter::table_eq("posts", "published", true))
    .where(Filter::table_eq("users", "active", true))
    .where(Filter::or(vec![
        Filter::table_eq("posts", "featured", true),
        Filter::and(vec![
            Filter::table_gt("posts", "created_at", Utc::now() - Duration::days(7)),
            Filter::table_gt("posts", "views", 1000),
        ]),
    ]))
    .order_by_desc(Some("posts"), "created_at")
    .limit(50)
    .fetch(provider, "db").await?;
```

---

## Best Practices

1. **Use entity-level joins** for common relationships, ad-hoc joins for specific queries
2. **Use column aliasing** (`columns` parameter) when selecting from joined tables to avoid SQL errors
3. **Leverage Filter combinators** (and/or/not) for complex conditions
4. **Use type-safe filters** - natural Rust types are automatically converted
5. **Implement FetchValue** for custom domain types (new types, enums)
6. **Use upserts** instead of "select then insert/update" patterns
7. **Always paginate** large result sets with limit/offset
8. **Prefer table-qualified filters** when working with joins to avoid ambiguity
9. **Handle column name conflicts** by aliasing one or both columns in the `columns` parameter
