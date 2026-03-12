# Identity

**When Required**: ANY HTTP call uses Bearer authentication.

For the trait definition and method signatures, see [capabilities.md](../capabilities.md). For provider composition rules, see [README.md](README.md).

---

## Production Patterns

The `Identity` trait provides access token retrieval via `Identity::access_token(provider, identity_name)`. Production Provider structs use empty implementations that delegate to the Omnia SDK defaults:

```rust
impl Identity for Provider {}
```

### The Authentication Pattern

When any HTTP call requires a Bearer token, the handler must include `Identity` in its bounds and follow this sequence:

```rust
// 1. Read identity name from config
let identity = Config::get(provider, "AZURE_IDENTITY").await?;

// 2. Fetch access token
let token = Identity::access_token(provider, identity).await?;

// 3. Attach token to HTTP request
let request = http::Request::builder()
    .header("Authorization", format!("Bearer {token}"))
    // ...
```

This means the handler bounds become `P: Config + Identity + HttpRequest` at minimum.

---

## MockProvider Implementation

### Basic Implementation

```rust
use omnia_sdk::Identity;

impl Identity for MockProvider {
    async fn access_token(&self, _identity: String) -> anyhow::Result<String> {
        Ok("mock_access_token_12345".to_string())
    }
}
```

### With Identity-Specific Tokens

```rust
impl Identity for MockProvider {
    async fn access_token(&self, identity: String) -> anyhow::Result<String> {
        match identity.as_str() {
            "azure-ad" => Ok("mock_azure_token".to_string()),
            "service-principal" => Ok("mock_sp_token".to_string()),
            _ => Ok(format!("mock_token_for_{}", identity)),
        }
    }
}
```

### With Token Validation

```rust
use std::sync::{Arc, Mutex};

#[derive(Clone)]
pub struct MockProvider {
    token_requests: Arc<Mutex<Vec<String>>>,
}

impl MockProvider {
    pub fn token_requests(&self) -> Vec<String> {
        self.token_requests.lock().unwrap().clone()
    }
}

impl Identity for MockProvider {
    async fn access_token(&self, identity: String) -> anyhow::Result<String> {
        self.token_requests.lock().unwrap().push(identity.clone());
        Ok(format!("mock_token_{}", identity))
    }
}

// Usage in tests:
let tokens = provider.token_requests();
assert_eq!(tokens, vec!["azure-ad"]);
```

### Best Practices

- Return realistic token format
- Track token requests for verification
- Support multiple identity types
- Don't return empty strings

## References

- [capabilities.md](../capabilities.md) -- Identity trait definition
- [http-request.md](http-request.md) -- HttpRequest mock patterns (often used together)
- [README.md](README.md) -- Provider composition rules
- [runtime.md](../runtime.md) -- Identity environment variables for local dev
