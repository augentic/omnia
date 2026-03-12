# HttpRequest

**When Required**: Component makes HTTP calls.

For the trait definition and method signatures, see [capabilities.md](../capabilities.md). For provider composition rules, see [README.md](README.md).

---

## Production Patterns

The `HttpRequest` trait provides HTTP client capabilities via `HttpRequest::fetch(provider, request)`. Production Provider structs use empty implementations that delegate to the Omnia SDK defaults:

```rust
impl HttpRequest for Provider {}
```

Usage in domain functions:

```rust
let request = http::Request::builder()
    .method("GET")
    .uri(format!("{api_url}/items/{id}"))
    .body(Empty::<Bytes>::new())?;
let response = HttpRequest::fetch(provider, request).await?;
```

When HTTP calls require authentication, combine with the [Identity](identity.md) trait. See [The Authentication Pattern](README.md#the-authentication-pattern).

---

## MockProvider Implementation

### Basic Implementation

```rust
use omnia_sdk::HttpRequest;
use http::Response;
use bytes::Bytes;

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

        // Match on URI patterns
        if uri.contains("/api/users") {
            let body = serde_json::json!({
                "id": 1,
                "name": "Test User"
            });
            Ok(http::Response::builder()
                .status(200)
                .header("Content-Type", "application/json")
                .body(bytes::Bytes::from(serde_json::to_vec(&body)?))?)
        } else {
            Ok(http::Response::builder()
                .status(404)
                .body(bytes::Bytes::from("Not Found"))?)
        }
    }
}
```

### With URI Pattern Matching

```rust
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
        let base = "https://example.com";

        // List endpoint
        if uri == format!("{base}/api/items") {
            let body = serde_json::json!([
                {"id": 1, "name": "Item 1"},
                {"id": 2, "name": "Item 2"}
            ]);
            return Ok(http::Response::builder()
                .status(200)
                .body(bytes::Bytes::from(serde_json::to_vec(&body)?))?);
        }

        // Single item endpoint with ID
        if uri.starts_with(&format!("{base}/api/items/")) {
            let id_str = uri.trim_start_matches(&format!("{base}/api/items/"));
            let id: u32 = id_str.parse().unwrap_or(0);

            if id > 0 && id <= 100 {
                let body = serde_json::json!({
                    "id": id,
                    "name": format!("Item {}", id)
                });
                return Ok(http::Response::builder()
                    .status(200)
                    .body(bytes::Bytes::from(serde_json::to_vec(&body)?))?);
            } else {
                return Ok(http::Response::builder()
                    .status(404)
                    .body(bytes::Bytes::from("Not Found"))?);
            }
        }

        // Default 404
        Ok(http::Response::builder()
            .status(404)
            .body(bytes::Bytes::from("Not Found"))?)
    }
}
```

### With Query Parameters

```rust
impl HttpRequest for MockProvider {
    async fn fetch<T>(
        &self, request: http::Request<T>,
    ) -> anyhow::Result<http::Response<bytes::Bytes>>
    where
        T: http_body::Body + std::any::Any + Send,
        T::Data: Into<Vec<u8>>,
        T::Error: Into<Box<dyn std::error::Error + Send + Sync + 'static>>,
    {
        let uri = request.uri();
        let path = uri.path();

        if path == "/api/search" {
            // Parse query parameters
            let query = uri.query().unwrap_or("");
            let params: std::collections::HashMap<_, _> = query
                .split('&')
                .filter_map(|pair| {
                    let (key, value) = pair.split_once('=')?;
                    Some((key, value))
                })
                .collect();

            // Filter results based on query
            let results = if let Some(user_id) = params.get("userId") {
                vec![
                    serde_json::json!({"id": 1, "userId": user_id}),
                    serde_json::json!({"id": 2, "userId": user_id}),
                ]
            } else {
                vec![]
            };

            Ok(http::Response::builder()
                .status(200)
                .body(bytes::Bytes::from(serde_json::to_vec(&results)?))?)
        } else {
            Ok(http::Response::builder()
                .status(404)
                .body(bytes::Bytes::from("Not Found"))?)
        }
    }
}
```

### With Embedded Test Data

```rust
impl MockProvider {
    fn load_users() -> Vec<User> {
        let data = include_bytes!("data/users.json");
        serde_json::from_slice(data).expect("should deserialize users")
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

        if uri.contains("/api/users") {
            let users = Self::load_users();
            let body = serde_json::to_vec(&users)?;
            Ok(http::Response::builder()
                .status(200)
                .body(bytes::Bytes::from(body))?)
        } else {
            Ok(http::Response::builder()
                .status(404)
                .body(bytes::Bytes::from("Not Found"))?)
        }
    }
}
```

### Best Practices

- Match on URI patterns, not exact strings
- Return realistic response structures
- Handle 404 for unknown endpoints
- Use embedded test data for complex responses
- Don't hardcode full URLs in matches

## References

- [capabilities.md](../capabilities.md) -- HttpRequest trait definition
- [identity.md](identity.md) -- Authentication pattern for Bearer tokens
- [README.md](README.md) -- Provider composition rules
