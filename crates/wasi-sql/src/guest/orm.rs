//! wasi-sql ORM layer for SQL databases.
//!
//! Provides a fluent API for building SQL queries with compile-time type safety,
//! automatic type conversions, and ``SeaQuery`` abstraction.
//!
//! # Quick Start
//!
//! ## Define an Entity
//!
//! ```ignore
//! use chrono::{DateTime, Utc};
//!
//! entity! {
//!     table = "posts",
//!     #[derive(Debug, Clone)]
//!     pub struct Post {
//!         pub id: i32,
//!         pub title: String,
//!         pub content: String,
//!         pub published: bool,
//!         pub created_at: DateTime<Utc>,
//!     }
//! }
//! ```
//!
//! ## CRUD Operations
//!
//! ```ignore
//! use qwasr_wasi_sql::orm::{SelectBuilder, InsertBuilder, UpdateBuilder, DeleteBuilder, Filter};
//!
//! // Select with filter
//! let posts = SelectBuilder::<Post>::new()
//!     .where(Filter::eq("published", true))
//!     .where(Filter::gt("created_at", Utc::now() - Duration::days(7)))
//!     .order_by_desc(None, "created_at")
//!     .limit(10)
//!     .fetch(provider, "db").await?;
//!
//! // Insert
//! InsertBuilder::<Post>::new()
//!     .set("title", "Hello World")
//!     .set("content", "My first post")
//!     .set("published", true)
//!     .build()?;
//!
//! // Or insert from an entity
//! let post = Post {
//!     id: 1,
//!     title: "Hello".to_string(),
//!     content: "World".to_string(),
//!     published: true,
//!     created_at: Utc::now(),
//! };
//! InsertBuilder::<Post>::from_entity(&post).build()?;
//!
//! // Update
//! UpdateBuilder::<Post>::new()
//!     .set("published", true)
//!     .where(Filter::eq("id", 42))
//!     .build()?;
//!
//! // Delete
//! DeleteBuilder::<Post>::new()
//!     .where(Filter::eq("id", 42))
//!     .build()?;
//! ```
//!
//! ## Joins
//!
//! ```ignore
//! use qwasr_wasi_sql::orm::Join;
//!
//! // Entity with default joins and column aliasing
//! entity! {
//!     table = "posts",
//!     columns = [
//!         ("users", "name", "author_name"),       // users.name AS author_name
//!         ("users", "email", "author_email"),    // users.email AS author_email
//!     ],
//!     joins = [
//!         Join::left("users", Filter::col_eq("posts", "author_id", "users", "id")),
//!     ],
//!     #[derive(Debug, Clone)]
//!     pub struct PostWithAuthor {
//!         pub id: i32,              // Auto: posts.id
//!         pub title: String,        // Auto: posts.title
//!         pub author_name: String,  // Manual: users.name AS author_name
//!         pub author_email: String, // Manual: users.email AS author_email
//!     }
//! }
//!
//! // Joins happen automatically
//! let posts = SelectBuilder::<PostWithAuthor>::new()
//!     .fetch(provider, "db").await?;
//!
//! // Or add ad-hoc joins
//! let posts = SelectBuilder::<Post>::new()
//!     .join(Join::left("users", Filter::col_eq("posts", "author_id", "users", "id")))
//!     .fetch(provider, "db").await?;
//! ```
//!
//! ## Filtering
//!
//! ```ignore
//! // Basic comparisons
//! Filter::eq("status", "active")
//! Filter::gt("views", 1000)
//! Filter::like("title", "%rust%")
//! Filter::in("id", vec![1, 2, 3])
//!
//! // Logical combinators
//! Filter::and(vec![
//!     Filter::eq("published", true),
//!     Filter::gt("views", 100),
//! ])
//!
//! Filter::or(vec![
//!     Filter::eq("featured", true),
//!     Filter::gt("views", 5000),
//! ])
//!
//! // Table-qualified (for joins)
//! Filter::table_eq("posts", "published", true)
//! Filter::col_eq("posts", "author_id", "users", "id")
//! ```
//!
//! ## Upserts
//!
//! Handle INSERT ... ON CONFLICT scenarios using native database syntax (PostgreSQL/SQLite).
//! These are **atomic operations** - the database handles them in a single statement,
//! not as separate SELECT/INSERT/UPDATE queries.
//!
//! ```ignore
//! // Insert or update on conflict - atomic operation
//! InsertBuilder::<User>::new()
//!     .set("email", "test@example.com")
//!     .set("name", "John Doe")
//!     .on_conflict("email")
//!     .do_update(&["name"])
//!     .build()?;
//! // Generates: INSERT INTO users (email, name) VALUES ($1, $2)
//! //            ON CONFLICT (email) DO UPDATE SET name = EXCLUDED.name
//! ```
//!
//! ## Custom Types
//!
//! ```ignore
//! impl FetchValue for UserId {
//!     fn fetch(row: &Row, col: &str) -> anyhow::Result<Self> {
//!         let id: String = FetchValue::fetch(row, col)?;
//!         Ok(UserId(id))
//!     }
//! }
//! ```
//!
//! For more information, please refer to [`usage.md`](orm/usage.md).

mod delete;
mod entity;
mod filter;
mod insert;
mod join;
mod query;
mod select;
mod update;

use anyhow::{Result, anyhow};
pub use delete::DeleteBuilder;
pub use entity::{Entity, EntityValues, FetchValue};
pub use filter::Filter;
use futures::FutureExt;
use futures::future::BoxFuture;
pub use insert::InsertBuilder;
pub use join::Join;
pub use select::SelectBuilder;
pub use update::UpdateBuilder;

use crate::readwrite;
use crate::types::{Connection, DataType, Row, Statement};

/// Type alias for boxed futures returning ORM results.
pub type FutureResult<T> = BoxFuture<'static, Result<T>>;

/// Trait for types that provide ORM database access.
///
/// Implement this trait to enable ORM operations. Default implementations
/// use the WASI SQL bindings to execute queries.
pub trait TableStore: Send + Sync {
    /// Executes a query and returns the result rows.
    ///
    /// # Errors
    ///
    /// Returns an error if the connection fails, statement preparation fails, or query execution fails.
    fn query(
        &self, cnn_name: String, query: String, params: Vec<DataType>,
    ) -> FutureResult<Vec<Row>> {
        async {
            let cnn = Connection::open(cnn_name)
                .await
                .map_err(|e| anyhow!("failed to open connection: {}", e.trace()))?;

            let stmt = Statement::prepare(query, params)
                .await
                .map_err(|e| anyhow!("failed to prepare statement: {}", e.trace()))?;

            let res = readwrite::query(&cnn, &stmt)
                .await
                .map_err(|e| anyhow!("query failed: {}", e.trace()))?;

            Ok(res)
        }
        .boxed()
    }

    /// Executes a statement and returns the number of affected rows.
    ///
    /// # Errors
    ///
    /// Returns an error if the connection fails, statement preparation fails, or execution fails.
    fn exec(&self, cnn_name: String, query: String, params: Vec<DataType>) -> FutureResult<u32> {
        async {
            let cnn = Connection::open(cnn_name)
                .await
                .map_err(|e| anyhow!("failed to open connection: {}", e.trace()))?;

            let stmt = Statement::prepare(query, params)
                .await
                .map_err(|e| anyhow!("failed to prepare statement: {}", e.trace()))?;

            let res = readwrite::exec(&cnn, &stmt)
                .await
                .map_err(|e| anyhow!("exec failed: {}", e.trace()))?;

            Ok(res)
        }
        .boxed()
    }
}
