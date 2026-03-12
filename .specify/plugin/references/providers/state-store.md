# StateStore

**When Required**: Component uses caching or key-value storage.

For the trait definition and method signatures, see [capabilities.md](../capabilities.md). For provider composition rules, see [README.md](README.md).

---

## Production Patterns

The `StateStore` trait provides key-value storage via `StateStore::get`, `StateStore::set`, and `StateStore::delete`. Production Provider structs use empty implementations that delegate to the Omnia SDK defaults:

```rust
impl StateStore for Provider {}
```

Usage in domain functions:

```rust
// Read from cache
if let Some(cached) = StateStore::get(provider, &cache_key).await? {
    return Ok(serde_json::from_slice(&cached)?);
}

// Write to cache with TTL
let data = serde_json::to_vec(&result)?;
StateStore::set(provider, &cache_key, &data, Some(300)).await?;

// Delete from cache
StateStore::delete(provider, &cache_key).await?;
```

---

## MockProvider Implementation

### Complete Implementation

```rust
use omnia_sdk::StateStore;
use std::collections::HashMap;
use std::sync::Mutex;
use once_cell::sync::OnceCell;

static CACHE: OnceCell<Mutex<HashMap<String, Vec<u8>>>> = OnceCell::new();

pub fn cache() -> &'static Mutex<HashMap<String, Vec<u8>>> {
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

#[derive(Clone)]
pub struct MockProvider;

impl MockProvider {
    pub fn new() -> Self {
        cache(); // Initialize cache
        Self
    }
}

impl StateStore for MockProvider {
    async fn get(&self, key: &str) -> anyhow::Result<Option<Vec<u8>>> {
        let cache = cache()
            .lock()
            .map_err(|e| anyhow::anyhow!("cache lock poisoned: {e}"))?;
        Ok(cache.get(key).cloned())
    }

    async fn set(
        &self,
        key: &str,
        value: &[u8],
        _ttl_secs: Option<u64>,
    ) -> anyhow::Result<Option<Vec<u8>>> {
        let previous = cache()
            .lock()
            .map_err(|e| anyhow::anyhow!("cache lock poisoned: {e}"))?
            .insert(key.to_string(), value.to_vec());
        Ok(previous)
    }

    async fn delete(&self, key: &str) -> anyhow::Result<()> {
        cache()
            .lock()
            .map_err(|e| anyhow::anyhow!("cache lock poisoned: {e}"))?
            .remove(key);
        Ok(())
    }
}
```

### With TTL Tracking

```rust
use std::time::{SystemTime, Duration};

struct CacheEntry {
    value: Vec<u8>,
    expires_at: Option<SystemTime>,
}

static CACHE: OnceCell<Mutex<HashMap<String, CacheEntry>>> = OnceCell::new();

impl StateStore for MockProvider {
    async fn get(&self, key: &str) -> anyhow::Result<Option<Vec<u8>>> {
        let mut cache = cache().lock().unwrap();

        if let Some(entry) = cache.get(key) {
            // Check if expired
            if let Some(expires_at) = entry.expires_at {
                if SystemTime::now() > expires_at {
                    cache.remove(key);
                    return Ok(None);
                }
            }
            Ok(Some(entry.value.clone()))
        } else {
            Ok(None)
        }
    }

    async fn set(
        &self,
        key: &str,
        value: &[u8],
        ttl_secs: Option<u64>,
    ) -> anyhow::Result<Option<Vec<u8>>> {
        let expires_at = ttl_secs.map(|secs| {
            SystemTime::now() + Duration::from_secs(secs)
        });

        let entry = CacheEntry {
            value: value.to_vec(),
            expires_at,
        };

        let previous = cache()
            .lock()
            .unwrap()
            .insert(key.to_string(), entry)
            .map(|e| e.value);

        Ok(previous)
    }

    async fn delete(&self, key: &str) -> anyhow::Result<()> {
        cache().lock().unwrap().remove(key);
        Ok(())
    }
}
```

### With Cache Verification Helpers

```rust
impl MockProvider {
    pub fn cache_contains(&self, key: &str) -> bool {
        cache().lock().unwrap().contains_key(key)
    }

    pub fn cache_get<T: serde::de::DeserializeOwned>(&self, key: &str) -> Option<T> {
        let cache = cache().lock().unwrap();
        cache.get(key).and_then(|bytes| {
            serde_json::from_slice(bytes).ok()
        })
    }

    pub fn cache_clear(&self) {
        cache().lock().unwrap().clear();
    }
}

// Usage in tests:
assert!(provider.cache_contains("user:123"));
let user: User = provider.cache_get("user:123").unwrap();
assert_eq!(user.id, 123);
```

### Best Practices

- Use OnceCell for global cache state
- Handle lock poisoning errors
- Return previous value from set()
- Provide test helpers for cache verification
- Support TTL if component uses it
- Don't use static mut (unsafe)

## References

- [capabilities.md](../capabilities.md) -- StateStore trait definition
- [README.md](README.md) -- Provider composition rules
