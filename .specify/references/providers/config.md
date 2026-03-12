# Config

**When Required**: Always -- all components read environment variables.

For the trait definition and method signatures, see [capabilities.md](../capabilities.md). For provider composition rules, see [README.md](README.md).

---

## Production Patterns

The `Config` trait provides access to environment variables via `Config::get(provider, "KEY")`. Production Provider structs use empty implementations that delegate to the Omnia SDK defaults:

```rust
impl Config for Provider {}
```

Usage in domain functions:

```rust
let api_url = Config::get(provider, "API_URL").await?;
let timeout: u64 = Config::get(provider, "TIMEOUT_MS").await?.parse()?;
```

**Rule**: Always use `Config::get(provider, "KEY")`, never `std::env::var("KEY")`.

---

## MockProvider Implementation

### Basic Implementation

```rust
use omnia_sdk::Config;

#[derive(Clone)]
pub struct MockProvider;

impl Config for MockProvider {
    async fn get(&self, key: &str) -> anyhow::Result<String> {
        match key {
            "API_URL" => Ok("https://example.com".to_string()),
            "TIMEOUT_MS" => Ok("5000".to_string()),
            "MAX_RETRIES" => Ok("3".to_string()),
            _ => anyhow::bail!("unknown config key: {key}"),
        }
    }
}
```

### With Environment Variables

```rust
impl Config for MockProvider {
    async fn get(&self, key: &str) -> anyhow::Result<String> {
        // Allow override from actual env vars for integration tests
        if let Ok(value) = std::env::var(key) {
            return Ok(value);
        }

        // Fall back to mock values
        match key {
            "API_URL" => Ok("https://example.com".to_string()),
            "DATABASE_URL" => Ok("postgres://localhost/test".to_string()),
            _ => anyhow::bail!("unknown config key: {key}"),
        }
    }
}
```

### Best Practices

- Return all config keys used by component
- Use realistic mock values
- Return error for unknown keys (catches typos)
- Don't return empty strings for missing keys

## References

- [capabilities.md](../capabilities.md) -- Config trait definition
- [README.md](README.md) -- Provider composition and `ensure_env!` macro
