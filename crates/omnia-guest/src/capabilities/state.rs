//! Key-value state capability.
//!
//! The atomics (`cas` / `increment`) act on raw bucket bytes. A TTL-less `set`
//! stores raw bytes too, so those keys round-trip through `cas`; TTL-enveloped
//! values (`set` with `ttl_secs`) are not CAS targets.

use std::future::Future;

use anyhow::Result;

/// Typed CAS failure, mirroring the `wasi:keyvalue/atomics` cas-error variant.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum CasError {
    /// The stored value did not match `expected`; carries the value observed
    /// at conflict time (`None` = key absent). The caller decides whether to
    /// retry.
    #[error("cas conflict: the stored value did not match the expected value")]
    Conflict(Option<Vec<u8>>),
    /// The underlying store failed.
    #[error("store failure: {0}")]
    Store(String),
}

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

    /// One-shot compare-and-swap: replace the value at `key` only while the
    /// stored value matches `expected` (`None` means "key absent"). A mismatch
    /// fails with [`CasError::Conflict`]; there is no retry loop.
    #[cfg(not(target_arch = "wasm32"))]
    fn cas(
        &self, key: &str, expected: Option<&[u8]>, value: &[u8],
    ) -> impl Future<Output = Result<(), CasError>> + Send;

    /// Atomically increment the integer at `key` by `delta` (absent starts at
    /// zero), returning the new value.
    #[cfg(not(target_arch = "wasm32"))]
    fn increment(&self, key: &str, delta: i64) -> impl Future<Output = Result<i64>> + Send;

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

    /// One-shot compare-and-swap: replace the value at `key` only while the
    /// stored value matches `expected` (`None` means "key absent"). A mismatch
    /// fails with [`CasError::Conflict`]; there is no retry loop.
    #[cfg(target_arch = "wasm32")]
    fn cas(
        &self, key: &str, expected: Option<&[u8]>, value: &[u8],
    ) -> impl Future<Output = Result<(), CasError>> + Send {
        use omnia_wasi_keyvalue::atomics::{self, Cas};
        use omnia_wasi_keyvalue::store;

        async move {
            let bucket = store::open("cache".to_string())
                .await
                .map_err(|e| CasError::Store(format!("opening cache: {e:?}")))?;
            let cas = Cas::new(&bucket, key.to_string())
                .await
                .map_err(|e| CasError::Store(format!("creating cas handle: {e:?}")))?;
            let observed = cas
                .current()
                .await
                .map_err(|e| CasError::Store(format!("reading cas snapshot: {e:?}")))?;
            if observed.as_deref() != expected {
                return Err(CasError::Conflict(observed));
            }
            match atomics::swap(cas, value.to_vec()).await {
                Ok(()) => Ok(()),
                // A writer slipped in between `new` and `swap`: flatten the
                // fresh handle to its observed bytes, same typed conflict.
                Err(atomics::CasError::CasFailed(fresh)) => {
                    let observed = fresh
                        .current()
                        .await
                        .map_err(|e| CasError::Store(format!("reading fresh snapshot: {e:?}")))?;
                    Err(CasError::Conflict(observed))
                }
                Err(atomics::CasError::StoreError(e)) => {
                    Err(CasError::Store(format!("swapping `{key}`: {e:?}")))
                }
            }
        }
    }

    /// Atomically increment the integer at `key` by `delta` (absent starts at
    /// zero), returning the new value.
    #[cfg(target_arch = "wasm32")]
    fn increment(&self, key: &str, delta: i64) -> impl Future<Output = Result<i64>> + Send {
        use anyhow::Context;
        use omnia_wasi_keyvalue::{atomics, store};

        async move {
            let bucket = store::open("cache".to_string()).await.context("opening cache")?;
            atomics::increment(&bucket, key.to_string(), delta)
                .await
                .map_err(|e| anyhow::anyhow!("incrementing `{key}`: {e:?}"))
        }
    }
}
