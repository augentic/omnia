# TableStore Handler Patterns

This document covers the `TableStore` trait pattern used for database and table storage operations in Omnia business logic crates. `TableStore` covers **both** SQL databases **and** managed NoSQL table stores (Azure Table Storage, Azure Cosmos DB).

**Demonstrates:** `TableStore` and `Config` capability traits

## Overview

The `TableStore` trait provides data access for SQL databases and managed table stores. **Prefer the ORM layer** (SelectBuilder, InsertBuilder, UpdateBuilder, Filter) for CRUD and simple queries. Use **raw SQL** (TableStore::query / TableStore::exec) only for GeoSearch/spatial (e.g. PostGIS), nested subqueries, or complex transactional flows that appear in legacy code.

> **WARNING — The `omnia_wasi_sql` module name is misleading.** Despite the "sql" in the module name, `TableStore` is a general-purpose data access abstraction. The Omnia runtime provides native adapters for SQL databases, Azure Table Storage, Azure Cosmos DB, and other tabular/document stores behind this single trait. When migrating code that uses `@azure/data-tables`, `TableClient`, or `*.table.core.windows.net`, use `TableStore` — not `HttpRequest` or `StateStore`. Azure Table Storage being "NoSQL" is irrelevant to trait selection.

**Source:** `omnia_wasi_sql::orm::TableStore`

**Note:** The SDK re-exports `TableStore` as `omnia_sdk::TableStore` for wasm32 targets, but the canonical definition is in `omnia_wasi_sql::orm`.

## Trait Definition

```rust
pub trait TableStore: Send + Sync {
    /// Execute a query and return result rows.
    fn query(
        &self, cnn_name: String, query: String, params: Vec<DataType>,
    ) -> FutureResult<Vec<Row>>;

    /// Execute a statement and return the number of affected rows.
    fn exec(&self, cnn_name: String, query: String, params: Vec<DataType>) -> FutureResult<u32>;
}
```

For guest code, an empty `impl TableStore for Provider {}` is sufficient to use the default implementations that connect to WASI SQL resources.

## Entity Definition

Define database entities using the `entity!` macro:

```rust
use chrono::{DateTime, Utc};
use omnia_wasi_sql::entity;
use serde::{Deserialize, Serialize};

entity! {
    table = "users",
    #[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
    pub struct User {
        pub id: i32,
        pub name: String,
        pub email: String,
        pub active: bool,
        pub created_at: DateTime<Utc>,
    }
}

entity! {
    table = "articles",
    #[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
    pub struct Article {
        pub id: i32,
        pub title: String,
        pub content: String,
        pub author_id: i32,
        pub published: bool,
        pub view_count: i32,
        pub created_at: DateTime<Utc>,
        pub updated_at: DateTime<Utc>,
    }
}
```

The `entity!` macro generates ORM trait implementations automatically.

## Query Patterns

### SELECT with Filters

```rust
use omnia_wasi_sql::orm::{Filter, SelectBuilder, TableStore};

// Build SELECT query with filters
let mut builder = SelectBuilder::<User>::new()
    .limit(100)
    .order_by_desc(None, "created_at");

// Apply filter conditionally
if active_only {
    builder = builder.r#where(Filter::eq("active", true));
}

// Execute query against database
let users = builder
    .fetch(provider, &db_name)
    .await
    .map_err(|err| Error::from(anyhow::anyhow!("failed to fetch users: {err}")))?;
```

### SELECT by Primary Key

```rust
// Query for single item by id
let mut articles = SelectBuilder::<Article>::new()
    .r#where(Filter::eq("id", req.id))
    .fetch(provider, &db_name)
    .await
    .map_err(|err| Error::from(anyhow::anyhow!("query failed: {err}")))?;

// Extract single result or error
let article = articles
    .pop()
    .ok_or_else(|| Error::from(anyhow::anyhow!("article not found: {}", req.id)))?;
```

### SELECT with Multiple Filters

```rust
let mut builder = SelectBuilder::<Article>::new()
    .limit(u64::from(req.limit.unwrap_or(50)))
    .order_by_desc(None, "created_at");

// Apply published_only filter
if req.published_only {
    builder = builder.r#where(Filter::eq("published", true));
}

// Apply author_id filter if provided
if let Some(author_id) = req.author_id {
    builder = builder.r#where(Filter::eq("author_id", author_id));
}

let articles = builder.fetch(provider, &db_name).await?;
```

### SELECT with Joins

```rust
entity! {
    table = "articles",
    columns = [
        ("users", "name", "author_name"),
    ],
    joins = [
        Join::left("users", Filter::col_eq("articles", "author_id", "users", "id")),
    ],
    #[derive(Debug, Clone)]
    pub struct ArticleWithAuthor {
        pub id: i32,
        pub title: String,
        pub author_name: String,
    }
}

let articles = SelectBuilder::<ArticleWithAuthor>::new()
    .fetch(provider, "main-db")
    .await?;
```

### INSERT

```rust
use omnia_wasi_sql::orm::InsertBuilder;

let builder = InsertBuilder::<Article>::new()
    .set("title", req.title.as_str())
    .set("content", req.content.as_str())
    .set("author_id", req.author_id)
    .set("published", req.published)
    .set("view_count", 0)
    .set("created_at", Utc::now())
    .set("updated_at", Utc::now());

// Build and execute
let query = builder.build().map_err(|err| Error::from(anyhow::anyhow!("build failed: {err}")))?;

provider
    .exec(db_name, query.sql, query.params)
    .await
    .map_err(|err| Error::from(anyhow::anyhow!("insert failed: {err}")))?;
```

### UPDATE

```rust
use omnia_wasi_sql::orm::{Filter, UpdateBuilder};

let builder = UpdateBuilder::<Article>::new()
    .set("published", true)
    .set("updated_at", Utc::now())
    .r#where(Filter::eq("id", req.id));

let query = builder.build().map_err(|err| Error::from(anyhow::anyhow!("build failed: {err}")))?;

let rows_affected = provider
    .exec(db_name, query.sql, query.params)
    .await
    .map_err(|err| Error::from(anyhow::anyhow!("update failed: {err}")))?;
```

### DELETE

```rust
use omnia_wasi_sql::orm::{DeleteBuilder, Filter};

DeleteBuilder::<Article>::new()
    .r#where(Filter::eq("id", 42))
    .execute(provider, "main-db")
    .await?;
```

## Complete Handler Examples

### List Handler with Filters

```rust
use anyhow::Context as _;
use omnia_sdk::{bad_request, Config, Context, Error, Handler, Reply, Result};
use omnia_wasi_sql::orm::{Filter, SelectBuilder, TableStore};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct UserListRequest {
    pub active_only: Option<bool>,
    pub limit: Option<u32>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct UserListResponse {
    pub items: Vec<User>,
}

async fn fetch_user_list<P: Config + TableStore>(
    _owner: &str, provider: &P, req: UserListRequest,
) -> Result<UserListResponse> {
    // Validate limit parameter
    if let Some(limit) = req.limit {
        if limit == 0 {
            return Err(bad_request!("limit must be greater than 0"));
        }
        if limit > 1000 {
            return Err(bad_request!("limit must be <= 1000"));
        }
    }

    // Get database name from config
    let db_name = Config::get(provider, "DATABASE_NAME")
        .await
        .context("getting DATABASE_NAME")?;

    // Build SELECT query with filters
    let mut builder = SelectBuilder::<User>::new()
        .limit(u64::from(req.limit.unwrap_or(100)))
        .order_by_desc(None, "created_at");

    // Apply active_only filter if requested
    if req.active_only.unwrap_or(false) {
        builder = builder.r#where(Filter::eq("active", true));
    }

    // Execute query against database
    let users = builder
        .fetch(provider, &db_name)
        .await
        .map_err(|err| Error::from(anyhow::anyhow!("failed to fetch users: {err}")))?;

    Ok(UserListResponse { items: users })
}

impl<P: Config + TableStore> Handler<P> for UserListRequest {
    type Error = Error;
    type Input = Vec<u8>;
    type Output = UserListResponse;

    async fn handle(self, ctx: Context<'_, P>) -> Result<Reply<UserListResponse>> {
        Ok(fetch_user_list(ctx.owner, ctx.provider, self).await?.into())
    }

    fn from_input(input: Self::Input) -> Result<Self> {
        serde_json::from_slice(&input)
            .context("deserializing UserListRequest")
            .map_err(Into::into)
    }
}
```

### Single Item Handler

```rust
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ArticleRequest {
    pub id: i32,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ArticleResponse {
    pub item: Article,
}

async fn fetch_article<P: Config + TableStore>(
    _owner: &str, provider: &P, req: ArticleRequest,
) -> Result<ArticleResponse> {
    if req.id <= 0 {
        return Err(bad_request!("article id must be positive"));
    }

    let db_name = Config::get(provider, "DATABASE_NAME")
        .await
        .context("getting DATABASE_NAME")?;

    let mut articles = SelectBuilder::<Article>::new()
        .r#where(Filter::eq("id", req.id))
        .fetch(provider, &db_name)
        .await
        .map_err(|err| Error::from(anyhow::anyhow!("query failed: {err}")))?;

    articles
        .pop()
        .ok_or_else(|| Error::from(anyhow::anyhow!("article not found: {}", req.id)))
        .map(|item| ArticleResponse { item })
}

impl<P: Config + TableStore> Handler<P> for ArticleRequest {
    type Error = Error;
    type Input = Vec<u8>;
    type Output = ArticleResponse;

    async fn handle(self, ctx: Context<'_, P>) -> Result<Reply<ArticleResponse>> {
        Ok(fetch_article(ctx.owner, ctx.provider, self).await?.into())
    }

    fn from_input(input: Self::Input) -> Result<Self> {
        serde_json::from_slice(&input)
            .context("deserializing ArticleRequest")
            .map_err(Into::into)
    }
}
```

### Create Handler (INSERT)

```rust
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ArticleCreateRequest {
    pub title: String,
    pub content: String,
    pub author_id: i32,
    pub published: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ArticleCreateResponse {
    pub success: bool,
    pub message: String,
}

async fn create_article<P: Config + TableStore>(
    _owner: &str, provider: &P, req: ArticleCreateRequest,
) -> Result<ArticleCreateResponse> {
    // Validate input
    if req.title.trim().is_empty() {
        return Err(bad_request!("title cannot be empty"));
    }
    if req.content.trim().is_empty() {
        return Err(bad_request!("content cannot be empty"));
    }
    if req.author_id <= 0 {
        return Err(bad_request!("author_id must be positive"));
    }

    let db_name = Config::get(provider, "DATABASE_NAME")
        .await
        .context("getting DATABASE_NAME")?;

    // Build INSERT query
    let builder = InsertBuilder::<Article>::new()
        .set("title", req.title.as_str())
        .set("content", req.content.as_str())
        .set("author_id", req.author_id)
        .set("published", req.published)
        .set("view_count", 0)
        .set("created_at", Utc::now())
        .set("updated_at", Utc::now());

    let query = builder.build()
        .map_err(|err| Error::from(anyhow::anyhow!("build failed: {err}")))?;

    provider
        .exec(db_name, query.sql, query.params)
        .await
        .map_err(|err| Error::from(anyhow::anyhow!("insert failed: {err}")))?;

    Ok(ArticleCreateResponse {
        success: true,
        message: "Article created successfully".to_string(),
    })
}

impl<P: Config + TableStore> Handler<P> for ArticleCreateRequest {
    type Error = Error;
    type Input = Vec<u8>;
    type Output = ArticleCreateResponse;

    async fn handle(self, ctx: Context<'_, P>) -> Result<Reply<ArticleCreateResponse>> {
        Ok(create_article(ctx.owner, ctx.provider, self).await?.into())
    }

    fn from_input(input: Self::Input) -> Result<Self> {
        serde_json::from_slice(&input)
            .context("deserializing ArticleCreateRequest")
            .map_err(Into::into)
    }
}
```

### Update Handler

```rust
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ArticlePublishRequest {
    pub id: i32,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ArticlePublishResponse {
    pub success: bool,
    pub rows_affected: u32,
}

async fn publish_article<P: Config + TableStore>(
    _owner: &str, provider: &P, req: ArticlePublishRequest,
) -> Result<ArticlePublishResponse> {
    if req.id <= 0 {
        return Err(bad_request!("article id must be positive"));
    }

    let db_name = Config::get(provider, "DATABASE_NAME")
        .await
        .context("getting DATABASE_NAME")?;

    // Build UPDATE query
    let builder = UpdateBuilder::<Article>::new()
        .set("published", true)
        .set("updated_at", Utc::now())
        .r#where(Filter::eq("id", req.id));

    let query = builder.build()
        .map_err(|err| Error::from(anyhow::anyhow!("build failed: {err}")))?;

    let rows_affected = provider
        .exec(db_name, query.sql, query.params)
        .await
        .map_err(|err| Error::from(anyhow::anyhow!("update failed: {err}")))?;

    Ok(ArticlePublishResponse {
        success: rows_affected > 0,
        rows_affected,
    })
}

impl<P: Config + TableStore> Handler<P> for ArticlePublishRequest {
    type Error = Error;
    type Input = Vec<u8>;
    type Output = ArticlePublishResponse;

    async fn handle(self, ctx: Context<'_, P>) -> Result<Reply<ArticlePublishResponse>> {
        Ok(publish_article(ctx.owner, ctx.provider, self).await?.into())
    }

    fn from_input(input: Self::Input) -> Result<Self> {
        serde_json::from_slice(&input)
            .context("deserializing ArticlePublishRequest")
            .map_err(Into::into)
    }
}
```

## Azure Table Storage Examples

Azure Table Storage entities use the same `entity!` macro and ORM builders as SQL tables. The runtime adapter translates `SelectBuilder` queries into Azure Table Storage REST API calls internally.

### Entity Definition (Azure Table Storage)

Azure Table Storage entities include `PartitionKey` and `RowKey` as system properties. Define them as regular fields:

```rust
use omnia_wasi_sql::entity;
use serde::{Deserialize, Serialize};

entity! {
    table = "fleetdata",
    #[derive(Clone, Debug, Deserialize, Serialize)]
    pub struct RawVehicle {
        #[serde(rename(deserialize = "PartitionKey"))]
        pub partition_key: String,
        #[serde(rename(deserialize = "RowKey"))]
        pub row_key: String,
        #[serde(rename(deserialize = "Vehicle_Label"))]
        pub vehicle_label: Option<String>,
        #[serde(rename(deserialize = "Vehicle_Type"))]
        pub vehicle_type: Option<String>,
        #[serde(rename(deserialize = "Seating_Capacity"))]
        pub seating_capacity: Option<String>,
        #[serde(rename(deserialize = "Standing_Capacity"))]
        pub standing_capacity: Option<String>,
        #[serde(rename(deserialize = "Tag"))]
        pub tag: Option<String>,
    }
}
```

### SELECT All Entities (Azure Table Storage)

```rust
use anyhow::Context as _;
use omnia_sdk::{Config, Error, Result, bad_gateway};
use omnia_wasi_sql::orm::{SelectBuilder, TableStore};

async fn fetch_all_vehicles<P>(provider: &P) -> Result<Vec<RawVehicle>>
where
    P: Config + TableStore,
{
    let db_name = Config::get(provider, "FLEET_TABLE_STORE")
        .await
        .context("getting FLEET_TABLE_STORE config")?;

    let vehicles = SelectBuilder::<RawVehicle>::new()
        .fetch(provider, &db_name)
        .await
        .map_err(|e| bad_gateway!("failed to fetch fleet data: {e}"))?;

    Ok(vehicles)
}
```

### SELECT with Filter (Azure Table Storage)

```rust
async fn fetch_vehicles_by_type<P>(provider: &P, vehicle_type: &str) -> Result<Vec<RawVehicle>>
where
    P: Config + TableStore,
{
    let db_name = Config::get(provider, "FLEET_TABLE_STORE")
        .await
        .context("getting FLEET_TABLE_STORE config")?;

    let vehicles = SelectBuilder::<RawVehicle>::new()
        .r#where(Filter::eq("Vehicle_Type", vehicle_type))
        .fetch(provider, &db_name)
        .await
        .map_err(|e| bad_gateway!("failed to fetch vehicles by type: {e}"))?;

    Ok(vehicles)
}
```

### Cache-Aside with Azure Table Storage

When the legacy component loads data from Azure Table Storage on startup into an in-memory cache, the WASM translation is on-demand cache-aside: `StateStore` for caching + `TableStore` as the data source. See [statestore.md](./statestore.md#cache-aside-with-tablestore-on-demand-loading) for the complete cache-aside pattern.

```rust
async fn load_fleet_data<P>(provider: &P) -> Result<Vec<RawVehicle>>
where
    P: Config + TableStore + StateStore,
{
    // 1. Check StateStore cache
    if let Some(cached) = StateStore::get(provider, "fleet_api:fleet_data").await? {
        if let Ok(vehicles) = serde_json::from_slice::<Vec<RawVehicle>>(&cached) {
            return Ok(vehicles);
        }
    }

    // 2. Cache miss — fetch from TableStore (Azure Table Storage)
    let db_name = Config::get(provider, "FLEET_TABLE_STORE").await?;
    let vehicles = SelectBuilder::<RawVehicle>::new()
        .fetch(provider, &db_name)
        .await
        .map_err(|e| bad_gateway!("failed to fetch fleet data: {e}"))?;

    // 3. Populate cache with TTL
    if let Ok(serialized) = serde_json::to_vec(&vehicles) {
        let _ = StateStore::set(provider, "fleet_api:fleet_data", &serialized, Some(60)).await;
    }

    Ok(vehicles)
}
```

## Filter Reference

| Filter                | Description       | Example                                               |
| --------------------- | ----------------- | ----------------------------------------------------- |
| `Filter::eq`          | Equals            | `Filter::eq("status", "active")`                      |
| `Filter::ne`          | Not equals        | `Filter::ne("deleted", true)`                         |
| `Filter::gt`          | Greater than      | `Filter::gt("views", 100)`                            |
| `Filter::gte`         | Greater or equal  | `Filter::gte("rating", 4)`                            |
| `Filter::lt`          | Less than         | `Filter::lt("age", 18)`                               |
| `Filter::lte`         | Less or equal     | `Filter::lte("price", 99.99)`                         |
| `Filter::like`        | LIKE pattern      | `Filter::like("title", "%rust%")`                     |
| `Filter::in`          | IN list           | `Filter::in("id", vec![1, 2, 3])`                     |
| `Filter::is_null`     | IS NULL           | `Filter::is_null("deleted_at")`                       |
| `Filter::is_not_null` | IS NOT NULL       | `Filter::is_not_null("email")`                        |
| `Filter::and`         | AND combinator    | `Filter::and(vec![f1, f2])`                           |
| `Filter::or`          | OR combinator     | `Filter::or(vec![f1, f2])`                            |
| `Filter::not`         | NOT modifier      | `Filter::not(filter)`                                 |
| `Filter::table_eq`    | Table-qualified   | `Filter::table_eq("posts", "active", true)`           |
| `Filter::col_eq`      | Column comparison | `Filter::col_eq("posts", "author_id", "users", "id")` |

## Required Imports

```rust
// Entity definition
use omnia_wasi_sql::entity;

// ORM builders and filters
use omnia_wasi_sql::orm::{
    DeleteBuilder,
    Filter,
    InsertBuilder,
    SelectBuilder,
    TableStore,
    UpdateBuilder,
};

// SDK types
use omnia_sdk::{bad_request, Config, Context, Error, Handler, Reply, Result};

// Other common imports
use anyhow::Context as _;
use chrono::Utc;
use serde::{Deserialize, Serialize};
```

## Key Rules

1. **Target Architecture**: TableStore handlers are designed for `wasm32-wasip2` only
2. **Entity Macro**: Always use `entity!` macro for database models — SQL tables and Azure Table Storage entities alike
3. **Config for Database Name**: Get database/table store connection name from `Config` trait
4. **Validation First**: Validate input parameters before building queries
5. **Error Mapping**: Map ORM errors to `omnia_sdk::Error` with context
6. **Prefer ORM**: Use `SelectBuilder`, `InsertBuilder`, `UpdateBuilder`, `DeleteBuilder` and `Filter` for CRUD and simple queries. Use **raw SQL** (`TableStore::query` / `TableStore::exec`) only when legacy code requires:
   - **GeoSearch / spatial queries** (e.g. PostGIS `ST_AsText`, `ST_Simplify`, `ST_MakeLine`, geofence filters)
   - **Nested subqueries** or complex expressions that the ORM builders do not support
   - **Complex transactional** multi-statement flows
   When using raw SQL, always use parameterized queries (e.g. `?` placeholders and `DataType` params), never string-concatenate user input.
7. **Azure Table Storage uses TableStore**: When the source uses `@azure/data-tables`, `TableClient`, `listEntities`, or `*.table.core.windows.net`, use `TableStore` with ORM builders. Do NOT use `HttpRequest` for Azure Table Storage access — the runtime provides a native adapter. See the [Azure Table Storage Examples](#azure-table-storage-examples) section above.

## References

- See [../../references/sdk-api.md](../../references/sdk-api.md) for the Handler trait pattern
- See [../../references/capabilities.md](../../references/capabilities.md) for trait definitions
- See [../../references/providers.md](../../references/providers.md) for provider bound composition
- See [../../references/error-handling.md](../../references/error-handling.md) for error conventions
