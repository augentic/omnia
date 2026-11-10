//! Key-value state capability.
//!
//! The atomics (`cas` / `increment`) act on raw bucket bytes. A TTL-less `set`
//! stores raw bytes too, so those keys round-trip through `cas`; TTL-enveloped
//! values (`set` with `ttl_secs`) are not CAS targets.

use std::future::Future;

use anyhow::{Context as _, Result};
use serde::Serialize;
use serde::de::DeserializeOwned;

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
///
/// JSON codecs over the stored bytes are [`StateStoreExt`].
#[omnia_guest_macros::capability]
pub trait StateStore: Send + Sync {
    /// Retrieve a previously stored value from the state store.
    fn get(&self, key: &str) -> impl Future<Output = Result<Option<Vec<u8>>>> + Send;

    /// Store a value in the state store.
    fn set(
        &self, key: &str, value: &[u8], ttl_secs: Option<u64>,
    ) -> impl Future<Output = Result<()>> + Send;

    /// Delete a value from the state store.
    fn delete(&self, key: &str) -> impl Future<Output = Result<()>> + Send;

    /// One-shot compare-and-swap: replace the value at `key` only while the
    /// stored value matches `expected` (`None` means "key absent"). A mismatch
    /// fails with [`CasError::Conflict`]; there is no retry loop.
    fn cas(
        &self, key: &str, expected: Option<&[u8]>, value: &[u8],
    ) -> impl Future<Output = Result<(), CasError>> + Send;

    /// Atomically increment the integer at `key` by `delta` (absent starts at
    /// zero), returning the new value.
    fn increment(&self, key: &str, delta: i64) -> impl Future<Output = Result<i64>> + Send;
}

#[cfg(all(target_arch = "wasm32", target_os = "wasi"))]
impl StateStore for crate::WasiProvider {
    async fn get(&self, key: &str) -> Result<Option<Vec<u8>>> {
        let cache = open_cache().await?;
        cache.get(key).await.with_context(|| format!("reading `{key}`"))
    }

    async fn set(&self, key: &str, value: &[u8], ttl_secs: Option<u64>) -> Result<()> {
        let cache = open_cache().await?;
        cache.set(key, value, ttl_secs).await.map(drop).with_context(|| format!("writing `{key}`"))
    }

    async fn delete(&self, key: &str) -> Result<()> {
        let cache = open_cache().await?;
        cache.delete(key).await.with_context(|| format!("deleting `{key}`"))
    }

    async fn cas(&self, key: &str, expected: Option<&[u8]>, value: &[u8]) -> Result<(), CasError> {
        use omnia_wasi_keyvalue::atomics::{self, Cas};

        let bucket = open_bucket().await.map_err(|e| CasError::Store(format!("{e:#}")))?;
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
            // a writer slipped in between `new` and `swap`: the same typed conflict
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

    async fn increment(&self, key: &str, delta: i64) -> Result<i64> {
        use omnia_wasi_keyvalue::atomics;

        let bucket = open_bucket().await?;
        atomics::increment(&bucket, key.to_string(), delta)
            .await
            .map_err(|e| anyhow::anyhow!("incrementing `{key}`: {e:?}"))
    }
}

#[cfg(all(target_arch = "wasm32", target_os = "wasi"))]
async fn open_cache() -> Result<omnia_wasi_keyvalue::cache::Cache> {
    omnia_wasi_keyvalue::cache::open(crate::keyvalue::BUCKET)
        .await
        .with_context(|| format!("opening bucket `{}`", crate::keyvalue::BUCKET))
}

#[cfg(all(target_arch = "wasm32", target_os = "wasi"))]
async fn open_bucket() -> Result<omnia_wasi_keyvalue::store::Bucket> {
    omnia_wasi_keyvalue::store::open(crate::keyvalue::BUCKET.to_string())
        .await
        .with_context(|| format!("opening bucket `{}`", crate::keyvalue::BUCKET))
}

/// JSON codecs over any [`StateStore`].
///
/// Blanket-implemented for every [`StateStore`]; never implemented by hand.
pub trait StateStoreExt: StateStore {
    /// Retrieve and deserialize a JSON value from the state store.
    fn get_json<T: DeserializeOwned>(
        &self, key: &str,
    ) -> impl Future<Output = Result<Option<T>>> + Send {
        async move {
            let Some(bytes) = self.get(key).await? else {
                return Ok(None);
            };
            serde_json::from_slice(&bytes).with_context(|| format!("parsing `{key}`")).map(Some)
        }
    }

    /// Serialize `value` as JSON and store it in the state store.
    fn set_json<T: Serialize + Sync + ?Sized>(
        &self, key: &str, value: &T, ttl_secs: Option<u64>,
    ) -> impl Future<Output = Result<()>> + Send {
        async move {
            let bytes =
                serde_json::to_vec(value).with_context(|| format!("serializing `{key}`"))?;
            self.set(key, &bytes, ttl_secs).await
        }
    }
}

impl<P: StateStore + ?Sized> StateStoreExt for P {}
