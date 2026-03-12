# Cache Handler Example

This document contains a complete cache handler implementation from the `ex-cache` crate, demonstrating manual caching with the `StateStore` provider.

**Demonstrates:** `StateStore`, `Config`, and `HttpRequest` capability traits

## Overview

Unlike simple handlers, caching handlers:

- Check cache before making upstream requests (cache-first strategy)
- Use `StateStore` provider for explicit caching
- Transform data before caching (not just raw responses)
- Require multiple providers: `Config`, `HttpRequest`, `StateStore`

## Complete Implementation

```rust
//! Handlers for fetching post items from an upstream API with caching.
//!
//! ## Key Patterns
//!
//! - **Cache-first strategy**: Check cache before making upstream request
//! - **Manual cache management**: Use `StateStore` provider for explicit caching
//! - **Data transformation**: Cache transformed/enriched data, not raw responses
//! - **Multiple providers**: Requires `Config`, `HttpRequest`, and `StateStore`

use anyhow::Context as _;
use bytes::Bytes;
use omnia_sdk::{
    Config, Context, Error, Handler, HttpRequest, Reply, Result, StateStore,
    bad_gateway, server_error,
};
use serde::{Deserialize, Serialize};

use crate::types::{Post, RawPost};

/// Request for fetching a post item by ID.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct PostRequest {
    pub id: u32,
}

/// Response for fetching a post item.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct PostResponse {
    pub item: Post,
}

/// A handler that fetches a post item by ID from the upstream API.
///
/// The handler implements a cache-first strategy:
/// 1. Check if post exists in cache
/// 2. If cached, return immediately
/// 3. If not cached, fetch from upstream, transform, and cache
///
/// # Errors
/// * If the configuration and request cannot be used to build a valid URL.
/// * Fails if the upstream request fails or returns a non-200 status code.
/// * Fails if caching operations fail.
async fn fetch_post<P>(_owner: &str, provider: &P, req: PostRequest) -> Result<PostResponse>
where
    P: Config + HttpRequest + StateStore,
{
    // Step 1: Check cache first
    let cache_key = format!("post-{}", req.id);
    let cached_post = StateStore::get(provider, &cache_key)
        .await?
        .and_then(|data| serde_json::from_slice(&data).ok());

    if let Some(item) = cached_post {
        return Ok(PostResponse { item });
    }

    // Step 2: Not in cache, fetch from upstream API
    let base_url = Config::get(provider, "PROXY_URI").await?;
    let url = format!("{base_url}/posts/{}", req.id);

    let http_request = http::Request::builder()
        .method(http::Method::GET)
        .uri(url)
        .header("Accept", "application/json")
        .body(http_body_util::Empty::<Bytes>::new())
        .map_err(|err| server_error!("failed to build HTTP request: {err}"))?;

    let response = HttpRequest::fetch(provider, http_request)
        .await
        .map_err(|err| bad_gateway!("request failed: {err}"))?;

    // Step 3: Parse and transform the response
    let raw_post: RawPost = serde_json::from_slice(response.body())
        .map_err(|err| server_error!("failed to parse response: {err}"))?;
    let item: Post = raw_post.into();

    // Step 4: Cache the transformed post for future requests
    let serialized = serde_json::to_vec(&item)
        .map_err(|err| server_error!("failed to serialize post for caching: {err}"))?;
    StateStore::set(provider, &cache_key, &serialized, None)
        .await
        .map_err(|err| server_error!("failed to cache post: {err}"))?;

    Ok(PostResponse { item })
}

/// Handler trait implementation.
impl<P> Handler<P> for PostRequest
where
    P: Config + HttpRequest + StateStore,
{
    type Error = Error;
    type Input = Vec<u8>;
    type Output = PostResponse;

    async fn handle(self, ctx: Context<'_, P>) -> Result<Reply<PostResponse>> {
        Ok(fetch_post(ctx.owner, ctx.provider, self).await?.into())
    }

    fn from_input(input: Self::Input) -> Result<Self> {
        serde_json::from_slice(&input)
            .context("deserializing PostRequest")
            .map_err(Into::into)
    }
}
```

## List Handler with Bulk Caching

```rust
/// Request for fetching a list of post items optionally filtered by author ID.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PostListRequest {
    pub user_id: Option<u32>,
}

/// Response for fetching a list of post items.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct PostListResponse {
    pub items: Vec<Post>,
}

/// A handler that fetches a list of post items with individual caching.
///
/// Instead of caching the entire list, this handler:
/// 1. Fetches the list from upstream
/// 2. For each item, checks if it's already cached
/// 3. If cached, uses the cached version
/// 4. If not cached, transforms and caches the item
///
/// This ensures individual items are always fresh when accessed directly.
async fn fetch_post_list<P>(
    _owner: &str,
    provider: &P,
    req: PostListRequest,
) -> Result<PostListResponse>
where
    P: Config + HttpRequest + StateStore,
{
    // Build URL with optional filter
    let base_url = Config::get(provider, "PROXY_URI").await?;
    let url = req.user_id.map_or_else(
        || format!("{base_url}/posts"),
        |user_id| format!("{base_url}/posts?userId={user_id}"),
    );

    // Fetch from upstream
    let http_request = http::Request::builder()
        .method(http::Method::GET)
        .uri(url)
        .header("Accept", "application/json")
        .body(http_body_util::Empty::<Bytes>::new())
        .map_err(|err| server_error!("failed to build HTTP request: {err}"))?;

    let response = HttpRequest::fetch(provider, http_request)
        .await
        .map_err(|err| bad_gateway!("request failed: {err}"))?;

    let raw_posts: Vec<RawPost> = serde_json::from_slice(response.body())
        .map_err(|err| server_error!("failed to parse response: {err}"))?;

    // Process each item with caching
    let mut items = Vec::with_capacity(raw_posts.len());
    for raw_post in raw_posts {
        let cache_key = format!("post-{}", raw_post.id);

        // Check cache first
        let cached_post = StateStore::get(provider, &cache_key)
            .await?
            .and_then(|data| serde_json::from_slice(&data).ok());

        if let Some(item) = cached_post {
            items.push(item);
            continue;
        }

        // Transform and cache
        let item: Post = raw_post.into();
        let serialized = serde_json::to_vec(&item)
            .map_err(|err| server_error!("failed to serialize post for caching: {err}"))?;
        StateStore::set(provider, &cache_key, &serialized, None)
            .await
            .map_err(|err| server_error!("failed to cache post: {err}"))?;

        items.push(item);
    }

    Ok(PostListResponse { items })
}

impl<P> Handler<P> for PostListRequest
where
    P: Config + HttpRequest + StateStore,
{
    type Error = Error;
    type Input = Vec<u8>;
    type Output = PostListResponse;

    async fn handle(self, ctx: Context<'_, P>) -> Result<Reply<PostListResponse>> {
        Ok(fetch_post_list(ctx.owner, ctx.provider, self).await?.into())
    }

    fn from_input(input: Self::Input) -> Result<Self> {
        serde_json::from_slice(&input)
            .context("deserializing PostListRequest")
            .map_err(Into::into)
    }
}
```

## Supporting Types

```rust
/// Raw post from upstream API.
#[derive(Clone, Debug, Deserialize)]
pub struct RawPost {
    pub id: u32,
    #[serde(rename = "userId")]
    pub user_id: u32,
    pub title: String,
    pub body: String,
}

/// Transformed post with additional computed fields.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Post {
    pub id: u32,
    pub user_id: u32,
    pub title: String,
    pub body: String,
    pub word_count: usize,
    pub summary: String,
}

impl From<RawPost> for Post {
    fn from(raw: RawPost) -> Self {
        let word_count = raw.body.split_whitespace().count();
        let summary = raw.body.chars().take(100).collect::<String>() + "...";

        Self {
            id: raw.id,
            user_id: raw.user_id,
            title: raw.title,
            body: raw.body,
            word_count,
            summary,
        }
    }
}
```

## Key Patterns Demonstrated

### 1. Cache-First Strategy

```rust
// Always check cache before upstream
if let Some(cached) = StateStore::get(provider, &key).await? {
    return Ok(cached);
}
// Only fetch if not cached
```

### 2. Cache Key Naming

```rust
// Use consistent, predictable key patterns
let cache_key = format!("post-{}", req.id);
let cache_key = format!("user-{}-posts", user_id);
```

### 3. Transform Before Caching

```rust
// Cache the transformed data, not raw response
let item: Post = raw_post.into();  // Transform
StateStore::set(provider, &key, &serde_json::to_vec(&item)?, None).await?;
```

### 4. TTL for Cache Entries

```rust
// Cache indefinitely (None)
StateStore::set(provider, &key, &data, None).await?;

// Cache for 1 hour (3600 seconds)
StateStore::set(provider, &key, &data, Some(3600)).await?;
```

### 5. Graceful Cache Failures

```rust
// For non-critical caching, log but don't fail
if let Err(err) = StateStore::set(provider, &key, &data, None).await {
    tracing::warn!("failed to cache: {err}");
}
```

## Cache-Aside with TableStore (On-Demand Loading)

When the legacy component loads data from a managed data store (e.g., Azure Table Storage) on startup into an in-memory cache, the WASM translation is **on-demand cache-aside**: `StateStore` for caching + `TableStore` as the data source. The handler fetches from the data store on cache miss — no separate cron/ETL component is needed.

```rust
use anyhow::Context as _;
use omnia_sdk::{
    Config, Context, Error, Handler, Reply, Result, StateStore,
    bad_gateway, server_error,
};
use omnia_wasi_sql::entity;
use omnia_wasi_sql::orm::{SelectBuilder, TableStore};
use serde::{Deserialize, Serialize};

const FLEET_CACHE_KEY: &str = "fleet-data";

entity! {
    table = "fleetdata",
    #[derive(Clone, Debug, Deserialize, Serialize)]
    pub struct RawVehicle {
        pub partition_key: String,
        pub row_key: String,
        pub label: Option<String>,
        pub call_sign: Option<String>,
    }
}

/// Load fleet data with cache-aside: StateStore first, TableStore on miss.
async fn load_fleet_data<P>(provider: &P) -> Result<Vec<RawVehicle>>
where
    P: Config + TableStore + StateStore,
{
    // 1. Check StateStore cache
    if let Some(bytes) = StateStore::get(provider, FLEET_CACHE_KEY)
        .await
        .map_err(|e| server_error!("reading fleet cache: {e}"))?
    {
        let vehicles: Vec<RawVehicle> = serde_json::from_slice(&bytes)
            .context("deserializing cached fleet data")
            .map_err(|e| server_error!("{e}"))?;
        return Ok(vehicles);
    }

    // 2. Cache miss — fetch from TableStore (Azure Table Storage)
    tracing::info!("Fleet data cache miss, fetching from TableStore");
    let db_name = Config::get(provider, "FLEET_TABLE_STORE")
        .await
        .map_err(|e| bad_gateway!("missing FLEET_TABLE_STORE config: {e}"))?;

    let vehicles = SelectBuilder::<RawVehicle>::new()
        .fetch(provider, &db_name)
        .await
        .map_err(|e| bad_gateway!("failed to fetch fleet data: {e}"))?;

    // 3. Populate cache with TTL (replaces legacy periodic refresh)
    if !vehicles.is_empty() {
        let ttl = Config::get(provider, "FLEET_CACHE_TTL_SECS")
            .await
            .ok()
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(60);

        let serialized = serde_json::to_vec(&vehicles)
            .context("serializing fleet data for cache")
            .map_err(|e| server_error!("{e}"))?;

        let _ = StateStore::set(provider, FLEET_CACHE_KEY, &serialized, Some(ttl))
            .await
            .map_err(|e| {
                tracing::warn!("Failed to cache fleet data: {e}");
                e
            });
    }

    Ok(vehicles)
}
```

### Key Differences from HttpRequest Cache-Aside

| Aspect | HttpRequest upstream | TableStore upstream |
|--------|---------------------|---------------------|
| Data source trait | `HttpRequest` | `TableStore` |
| Handler bounds | `P: Config + HttpRequest + StateStore` | `P: Config + TableStore + StateStore` |
| Fetch call | `HttpRequest::fetch(provider, request)` | `SelectBuilder::<T>::new().fetch(provider, &db_name)` |
| Auth | Explicit (Identity, API keys) | Handled by runtime |
| Use when | External REST APIs | Databases, Azure Table Storage, Cosmos DB |

## References

- See [../../references/sdk-api.md](../../references/sdk-api.md) for the Handler trait pattern
- See [../../references/capabilities.md](../../references/capabilities.md) for trait definitions
- See [../../references/providers.md](../../references/providers.md) for provider bound composition
- See [./tablestore.md](./tablestore.md) for TableStore ORM patterns
