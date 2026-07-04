//! Key-value state capability.

use std::future::Future;

use anyhow::Result;

/// Store and retrieve key-value state, optionally with a TTL.
pub trait StateStore: Send + Sync {
    /// Retrieve a previously stored value from the state store.
    #[cfg(not(target_arch = "wasm32"))]
    fn get(&self, key: &str) -> impl Future<Output = Result<Option<Vec<u8>>>> + Send;

    /// Store a value in the state store.
    #[cfg(not(target_arch = "wasm32"))]
    fn set(
        &self, key: &str, value: &[u8], ttl_secs: Option<u64>,
    ) -> impl Future<Output = Result<Option<Vec<u8>>>> + Send;

    /// Delete a value from the state store.
    #[cfg(not(target_arch = "wasm32"))]
    fn delete(&self, key: &str) -> impl Future<Output = Result<()>> + Send;

    /// Retrieve a previously stored value from the state store.
    #[cfg(target_arch = "wasm32")]
    fn get(&self, key: &str) -> impl Future<Output = Result<Option<Vec<u8>>>> + Send {
        use anyhow::Context;
        async move {
            let bucket =
                omnia_wasi_keyvalue::cache::open("cache").await.context("opening cache")?;
            bucket.get(key).await.context("reading state from cache")
        }
    }

    /// Store a value in the state store.
    #[cfg(target_arch = "wasm32")]
    fn set(
        &self, key: &str, value: &[u8], ttl_secs: Option<u64>,
    ) -> impl Future<Output = Result<Option<Vec<u8>>>> + Send {
        use anyhow::Context;
        async move {
            let bucket =
                omnia_wasi_keyvalue::cache::open("cache").await.context("opening cache")?;
            bucket.set(key, value, ttl_secs).await.context("writing state to cache")
        }
    }

    /// Delete a value from the state store.
    #[cfg(target_arch = "wasm32")]
    fn delete(&self, key: &str) -> impl Future<Output = Result<()>> + Send {
        use anyhow::Context;
        async move {
            let bucket =
                omnia_wasi_keyvalue::cache::open("cache").await.context("opening cache")?;
            bucket.delete(key).await.context("deleting entry from cache")
        }
    }
}
