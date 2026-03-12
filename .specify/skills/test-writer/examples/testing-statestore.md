# Example 02: Cache with StateStore

Complete working example demonstrating multi-trait MockProvider with StateStore, based on the `ex-cache` crate from the context workspace.

## Scenario

Generate test harness for a caching component that:
- Fetches data from external HTTP API
- Caches results in StateStore
- Implements Config, HttpRequest, and StateStore traits

## Component Structure

```
ex-cache/
├── src/
│   ├── lib.rs
│   ├── handlers.rs
│   └── types.rs
├── tests/
│   ├── provider.rs      # Multi-trait MockProvider
│   ├── post.rs          # Test cases
│   └── data/
│       ├── posts.json   # Mock HTTP response data
│       └── todos.json
└── Cargo.toml
```

## Handler Code (Reference)

### handlers.rs

```rust
use omnia_sdk::{Config, Handler, HttpRequest, StateStore};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct PostRequest {
    pub id: u32,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct PostResponse {
    pub item: Post,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Post {
    pub id: u32,
    pub user_id: u32,
    pub title: String,
    pub body: String,
    pub word_count: usize,
}

impl<P: Config + HttpRequest + StateStore> Handler<P> for PostRequest {
    type Response = PostResponse;

    async fn handle(self, provider: &P) -> anyhow::Result<Self::Response> {
        let cache_key = format!("post-{}", self.id);
        
        // Check cache first
        if let Some(cached) = provider.get(&cache_key).await? {
            let item: Post = serde_json::from_slice(&cached)?;
            return Ok(PostResponse { item });
        }
        
        // Fetch from API
        let base_url = provider.get("PROXY_URI").await?;
        let url = format!("{}/posts/{}", base_url, self.id);
        let request = http::Request::builder()
            .uri(url)
            .body(())?;
        
        let response = provider.fetch(request).await?;
        let raw_post: RawPost = serde_json::from_slice(&response.body())?;
        
        // Transform and cache
        let post = Post {
            id: raw_post.id,
            user_id: raw_post.user_id,
            title: raw_post.title.clone(),
            body: raw_post.body.clone(),
            word_count: raw_post.body.split_whitespace().count(),
        };
        
        let cached_data = serde_json::to_vec(&post)?;
        provider.set(&cache_key, &cached_data, Some(3600)).await?;
        
        Ok(PostResponse { item: post })
    }
}
```

## Generated Test Files

### tests/provider.rs

```rust
use std::collections::HashMap;
use std::sync::Mutex;

use ex_cache::types::{RawPost, Todo};
use once_cell::sync::OnceCell;
use omnia_sdk::{Config, HttpRequest, StateStore};

static CACHE: OnceCell<Mutex<HashMap<String, Vec<u8>>>> = OnceCell::new();

pub fn cache() -> &'static Mutex<HashMap<String, Vec<u8>>> {
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

#[derive(Clone)]
pub struct MockProvider;

impl Default for MockProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl MockProvider {
    #[must_use]
    pub fn new() -> Self {
        cache();
        Self
    }

    fn todos() -> Vec<Todo> {
        let todos_data = include_bytes!("data/todos.json");
        let todos: Vec<Todo> =
            serde_json::from_slice(todos_data).expect("should deserialize sample to-do items");
        todos
    }

    fn posts() -> Vec<RawPost> {
        let posts_data = include_bytes!("data/posts.json");
        let posts: Vec<RawPost> =
            serde_json::from_slice(posts_data).expect("should deserialize sample post items");
        posts
    }
}

impl HttpRequest for MockProvider {
    async fn fetch<T>(
        &self, request: http::Request<T>,
    ) -> anyhow::Result<http::Response<bytes::Bytes>>
    where
        T: http_body::Body + std::any::Any + Send,
        T::Data: Into<Vec<u8>>,
        T::Error: Into<Box<dyn std::error::Error + Send + Sync + 'static>>,
    {
        let uri = request.uri().to_string();
        let base = Config::get(self, "PROXY_URI").await?;
        let todo_list_uri = format!("{base}/todos");
        let todo_uri = format!("{base}/todos/");
        let post_list_uri = format!("{base}/posts");
        let post_uri = format!("{base}/posts/");

        let response = if uri.starts_with(&todo_uri) {
            let id_str = uri.trim_start_matches(&todo_uri);
            let id: u32 = id_str.parse().unwrap_or_default();
            let todos = Self::todos();
            let todo = todos.into_iter().find(|t| t.id == id);
            if let Some(todo) = todo {
                let body = serde_json::to_vec(&todo)?;
                http::Response::builder()
                    .status(200)
                    .header("Content-Type", "application/json")
                    .body(bytes::Bytes::from(body))?
            } else {
                http::Response::builder().status(404).body(bytes::Bytes::from("Not Found"))?
            }
        } else if uri.starts_with(&todo_list_uri) {
            let todos = Self::todos();
            let body = serde_json::to_vec(&todos)?;
            http::Response::builder()
                .status(200)
                .header("Content-Type", "application/json")
                .body(bytes::Bytes::from(body))?
        } else if uri.starts_with(&post_uri) {
            let id_str = uri.trim_start_matches(&post_uri);
            let id: u32 = id_str.parse().unwrap_or_default();
            let posts = Self::posts();
            let post = posts.into_iter().find(|p| p.id == id);
            if let Some(post) = post {
                let body = serde_json::to_vec(&post)?;
                http::Response::builder()
                    .status(200)
                    .header("Content-Type", "application/json")
                    .body(bytes::Bytes::from(body))?
            } else {
                http::Response::builder().status(404).body(bytes::Bytes::from("Not Found"))?
            }
        } else if uri.starts_with(&post_list_uri) {
            let all_posts = Self::posts();
            let posts = if let Some(user_id) = request.uri().query().and_then(|query| {
                query.split('&').find_map(|pair| {
                    let (key, value) = pair.split_once('=')?;
                    if key == "userId" { value.parse::<u32>().ok() } else { None }
                })
            }) {
                all_posts.into_iter().filter(|post| post.user_id == user_id).collect::<Vec<_>>()
            } else {
                all_posts
            };
            let body = serde_json::to_vec(&posts)?;
            http::Response::builder()
                .status(200)
                .header("Content-Type", "application/json")
                .body(bytes::Bytes::from(body))?
        } else {
            http::Response::builder().status(404).body(bytes::Bytes::from("Not Found"))?
        };

        Ok(response)
    }
}

impl Config for MockProvider {
    async fn get(&self, key: &str) -> anyhow::Result<String> {
        match key {
            "PROXY_URI" => Ok("https://example.com".to_string()),
            "CACHE_BUCKET" => Ok("test-bucket".to_string()),
            _ => anyhow::bail!("unknown config key: {key}"),
        }
    }
}

impl StateStore for MockProvider {
    async fn get(&self, key: &str) -> anyhow::Result<Option<Vec<u8>>> {
        let cache = cache().lock().map_err(|e| anyhow::anyhow!("cache lock poisoned: {e}"))?;
        Ok(cache.get(key).cloned())
    }

    async fn set(
        &self, key: &str, value: &[u8], _ttl_secs: Option<u64>,
    ) -> anyhow::Result<Option<Vec<u8>>> {
        let previous = cache()
            .lock()
            .map_err(|e| anyhow::anyhow!("cache lock poisoned: {e}"))?
            .insert(key.to_string(), value.to_vec());

        Ok(previous)
    }

    async fn delete(&self, key: &str) -> anyhow::Result<()> {
        cache().lock().map_err(|e| anyhow::anyhow!("cache lock poisoned: {e}"))?.remove(key);
        Ok(())
    }
}
```

### tests/post.rs

```rust
mod provider;

use ex_cache::handlers::{PostListRequest, PostRequest};
use ex_cache::types::Post;
use omnia_sdk::{Client, StateStore};

use crate::provider::MockProvider;

#[tokio::test]
async fn post_item_handler_populates_cache() {
    let provider = MockProvider::new();
    let client = Client::new("tester").provider(provider.clone());
    let request = PostRequest { id: 42 };

    // Request should fetch from upstream and populate the cache.
    let response = client.request(request).await.expect("post item request should succeed");
    assert_eq!(response.item.id, 42);
    assert_eq!(response.item.user_id, 5);
    assert_eq!(
        response.item.title,
        "commodi ullam sint et excepturi error explicabo praesentium voluptas"
    );
    assert_eq!(
        response.item.body,
        "odio fugit voluptatum ducimus earum autem est incidunt voluptatem\nodit reiciendis aliquam sunt sequi nulla dolorem\nnon facere repellendus voluptates quia\nratione harum vitae ut"
    );

    // Check that the cache was populated.
    let cached = StateStore::get(&provider, "post-42").await.expect("cache get should succeed");
    assert!(cached.is_some(), "cache should contain entry for post-42");
    let cached_post =
        serde_json::from_slice::<Post>(&cached.unwrap()).expect("cached post should deserialize");
    assert_eq!(cached_post.word_count, 25);
}

#[tokio::test]
async fn post_list_handler_returns_expected() {
    let client = Client::new("tester").provider(MockProvider::new());
    let request = PostListRequest { user_id: Some(3) };

    // Request should return only post items from the specific user.
    let response = client.request(request).await.expect("first fetch_post_list should succeed");
    assert_eq!(response.items.len(), 10);
    for post in &response.items {
        assert_eq!(post.user_id, 3);
    }
    assert_eq!(response.items[0].id, 21);
}

#[tokio::test]
async fn post_list_handler_no_user_id_returns_all() {
    let client = Client::new("tester").provider(MockProvider::new());
    let request = PostListRequest { user_id: None };
    // Request should return all post items.
    let response = client.request(request).await.expect("first fetch_post_list should succeed");
    assert_eq!(response.items.len(), 100);
    assert_eq!(response.items[0].id, 1);
}

#[tokio::test]
async fn post_item_handler_not_found() {
    let client = Client::new("tester").provider(MockProvider::new());
    let request = PostRequest { id: 9999 }; // Non-existent post ID
    let result = client.request(request).await;
    assert!(result.is_err(), "fetch_post should return error for non-existent post");
}
```

## Key Points

### 1. Multi-Trait MockProvider

The MockProvider implements **three traits**:
- `Config` - Environment configuration
- `HttpRequest` - External API calls
- `StateStore` - Caching layer

### 2. Global Cache State

```rust
static CACHE: OnceCell<Mutex<HashMap<String, Vec<u8>>>> = OnceCell::new();

pub fn cache() -> &'static Mutex<HashMap<String, Vec<u8>>> {
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}
```

**Why OnceCell + Mutex**:
- `OnceCell` - Thread-safe lazy initialization
- `Mutex` - Interior mutability for cache updates
- **Not** `static mut` (unsafe and discouraged)

### 3. StateStore Implementation

```rust
impl StateStore for MockProvider {
    async fn get(&self, key: &str) -> anyhow::Result<Option<Vec<u8>>> {
        let cache = cache().lock().map_err(|e| anyhow::anyhow!("cache lock poisoned: {e}"))?;
        Ok(cache.get(key).cloned())
    }

    async fn set(
        &self, key: &str, value: &[u8], _ttl_secs: Option<u64>,
    ) -> anyhow::Result<Option<Vec<u8>>> {
        let previous = cache()
            .lock()
            .map_err(|e| anyhow::anyhow!("cache lock poisoned: {e}"))?
            .insert(key.to_string(), value.to_vec());
        Ok(previous)
    }

    async fn delete(&self, key: &str) -> anyhow::Result<()> {
        cache().lock().map_err(|e| anyhow::anyhow!("cache lock poisoned: {e}"))?.remove(key);
        Ok(())
    }
}
```

**Critical details**:
- Returns `Option<Vec<u8>>` from `get()`
- Returns previous value from `set()`
- Handles lock poisoning errors
- TTL parameter accepted but ignored in tests

### 4. Cache Verification

```rust
// Verify cache was populated
let cached = StateStore::get(&provider, "post-42").await.expect("cache get should succeed");
assert!(cached.is_some(), "cache should contain entry for post-42");

// Deserialize and verify cached data
let cached_post = serde_json::from_slice::<Post>(&cached.unwrap())
    .expect("cached post should deserialize");
assert_eq!(cached_post.word_count, 25);
```

### 5. Embedded Test Data

```rust
fn posts() -> Vec<RawPost> {
    let posts_data = include_bytes!("data/posts.json");
    serde_json::from_slice(posts_data).expect("should deserialize sample post items")
}
```

Uses `include_bytes!()` to embed JSON data at compile time.

## Running Tests

```bash
# Run all tests
cargo test

# Run specific test
cargo test post_item_handler_populates_cache

# Run with output
cargo test -- --nocapture
```

### Expected Output

```
running 4 tests
test post_item_handler_populates_cache ... ok
test post_list_handler_returns_expected ... ok
test post_list_handler_no_user_id_returns_all ... ok
test post_item_handler_not_found ... ok

test result: ok. 4 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
```

## Advanced Patterns

### Cache Hit Test

```rust
#[tokio::test]
async fn post_item_handler_uses_cache() {
    let provider = MockProvider::new();
    let client = Client::new("tester").provider(provider.clone());
    
    // First request - cache miss
    let request1 = PostRequest { id: 42 };
    let response1 = client.request(request1).await.unwrap();
    
    // Manually modify cache
    let mut modified_post = response1.item.clone();
    modified_post.word_count = 999;
    let cached_data = serde_json::to_vec(&modified_post).unwrap();
    StateStore::set(&provider, "post-42", &cached_data, None).await.unwrap();
    
    // Second request - should use cache
    let request2 = PostRequest { id: 42 };
    let response2 = client.request(request2).await.unwrap();
    
    // Verify cached value was used
    assert_eq!(response2.item.word_count, 999);
}
```

### Cache Expiration Test (with TTL tracking)

```rust
// If you implement TTL tracking in MockProvider:
#[tokio::test]
async fn cache_entry_expires() {
    let provider = MockProvider::new();
    
    // Set with 1 second TTL
    StateStore::set(&provider, "test-key", b"test-value", Some(1)).await.unwrap();
    
    // Immediate get - should exist
    let value1 = StateStore::get(&provider, "test-key").await.unwrap();
    assert!(value1.is_some());
    
    // Wait for expiration
    tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;
    
    // Get after expiration - should be None
    let value2 = StateStore::get(&provider, "test-key").await.unwrap();
    assert!(value2.is_none());
}
```

## Troubleshooting

### Issue: Cache Lock Poisoned

**Cause**: Panic while holding lock

**Solution**: Ensure no panics in cache operations
```rust
let cache = cache().lock().map_err(|e| anyhow::anyhow!("cache lock poisoned: {e}"))?;
```

### Issue: Cache Not Persisting Between Tests

**Cause**: Each test gets fresh MockProvider instance but shares global cache

**Solution**: Clear cache between tests if needed
```rust
#[tokio::test]
async fn test_with_clean_cache() {
    // Clear cache at start
    provider::cache().lock().unwrap().clear();
    
    // Run test...
}
```

### Issue: HTTP Response Not Found

**Cause**: URI pattern doesn't match

**Solution**: Debug URI matching
```rust
let uri = request.uri().to_string();
println!("Requested URI: {}", uri);  // Add debug output
```

## Dependencies

### Cargo.toml

```toml
[dependencies]
omnia-sdk = "0.26"
serde = { version = "1", features = ["derive"] }
serde_json = "1"
anyhow = "1"
http = "1"
bytes = "1"

[dev-dependencies]
tokio = { version = "1", features = ["full"] }
once_cell = "1"
```

## Summary

This example demonstrates:
- ✅ **Multi-trait MockProvider** (Config + HttpRequest + StateStore)
- ✅ **Global cache state** with OnceCell + Mutex
- ✅ **Complete StateStore implementation**
- ✅ **Cache verification** in tests
- ✅ **Embedded test data** with include_bytes!()
- ✅ **URI pattern matching** for HTTP mocking
- ✅ **Query parameter handling**

**Total lines of test code**: ~200 lines (provider + tests)
**Setup time**: ~30 minutes
**Maintenance**: Moderate

## Next Steps

- For Publish examples, see [testing-publisher.md](testing-publisher.md)
- For time-sensitive components, see [replay-writer](../../../../rt/skills/replay-writer/SKILL.md)
- For migration guidance, see [replay-writer](../../../../rt/skills/replay-writer/SKILL.md)
