//! Default in-memory implementation for wasi-keyvalue
//!
//! This is a lightweight implementation for development use only.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, Weak};
use std::time::{Duration, Instant};

use anyhow::{Context as _, Result, anyhow};
use futures::FutureExt;
use moka::Expiry;
use moka::sync::Cache;
use omnia_core::Connect;
use tracing::instrument;

use crate::host::resource::{Bucket, FutureResult};
use crate::host::{Error, KeyValueClient};

#[expect(missing_docs, reason = "`FromEnv` has no docs")]
mod config {
    use fromenv::{FromEnv, ParseResult};

    /// Options used to connect to the in-memory key-value store.
    ///
    /// This struct is used to load connection options from environment variables.
    #[derive(Debug, Clone, FromEnv)]
    pub struct ConnectOptions {
        /// The buckets a guest may open (`KEYVALUE_BUCKETS`, comma-separated, default: `default`).
        #[env(from = "KEYVALUE_BUCKETS", default = "default", with = comma_separated)]
        pub buckets: Vec<String>,
    }

    impl Default for ConnectOptions {
        fn default() -> Self {
            Self {
                buckets: vec!["default".to_owned()],
            }
        }
    }

    #[expect(clippy::unnecessary_wraps, reason = "the signature `fromenv` parsers take")]
    fn comma_separated(value: &str) -> ParseResult<Vec<String>> {
        Ok(value
            .split(',')
            .map(str::trim)
            .filter(|name| !name.is_empty())
            .map(str::to_owned)
            .collect())
    }
}
pub use config::ConnectOptions;

impl omnia_core::FromEnv for ConnectOptions {
    fn load_env() -> Result<Self> {
        // `Self::from_env()` is the builder the `FromEnv` derive emits
        Self::from_env().finalize().context("issue loading connection options")
    }
}

/// Default implementation for `wasi:keyvalue`.
#[derive(Clone)]
pub struct KeyValueDefault {
    buckets: Arc<HashMap<String, InMemBucket>>,
}

impl std::fmt::Debug for KeyValueDefault {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("KeyValueDefault").finish_non_exhaustive()
    }
}

impl Connect for KeyValueDefault {
    type ConnectOptions = ConnectOptions;

    #[instrument]
    async fn connect_with(options: Self::ConnectOptions) -> Result<Self> {
        tracing::debug!("initializing in-memory key-value store");
        let buckets = options
            .buckets
            .into_iter()
            .map(|name| (name.clone(), InMemBucket::new(name)))
            .collect();
        Ok(Self {
            buckets: Arc::new(buckets),
        })
    }
}

impl KeyValueClient for KeyValueDefault {
    fn open_bucket(&self, identifier: String) -> FutureResult<Arc<dyn Bucket>> {
        tracing::trace!("opening bucket: {identifier}");
        let bucket = self.buckets.get(&identifier).cloned().ok_or_else(|| {
            anyhow::Error::from(Error::NotFound(format!("bucket `{identifier}` is not declared")))
        });
        async move { Ok(Arc::new(bucket?) as Arc<dyn Bucket>) }.boxed()
    }
}

#[derive(Clone)]
struct InMemBucket {
    name: String,
    cache: BucketCache,
    locks: KeyLocks,
}

impl InMemBucket {
    fn new(name: String) -> Self {
        Self {
            name,
            cache: Cache::builder().expire_after(Deadline).build(),
            locks: Arc::default(),
        }
    }

    // Per-key, so writes and atomics serialize per key rather than per bucket.
    // `Weak` refs let unused keys drop their mutex; an entry is never removed
    // while a strong ref exists, or a concurrent call could install a second
    // mutex for the key.
    fn key_lock(&self, key: &str) -> Arc<Mutex<()>> {
        let mut locks = self.locks.lock().expect("lock registry poisoned");
        if let Some(existing) = locks.get(key).and_then(Weak::upgrade) {
            return existing;
        }
        locks.retain(|_, weak| weak.strong_count() > 0);
        let lock = Arc::new(Mutex::new(()));
        locks.insert(key.to_string(), Arc::downgrade(&lock));
        lock
    }
}

impl std::fmt::Debug for InMemBucket {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InMemBucket").field("name", &self.name).finish_non_exhaustive()
    }
}

impl Bucket for InMemBucket {
    fn get(&self, key: String) -> FutureResult<Option<Vec<u8>>> {
        tracing::trace!("getting key: {key} from bucket: {}", self.name);
        let result = self.cache.get(&key).map(|entry| entry.bytes);
        async move { Ok(result) }.boxed()
    }

    fn set(&self, key: String, value: Vec<u8>, ttl: Option<Duration>) -> FutureResult<()> {
        tracing::trace!("setting key: {key} in bucket: {}", self.name);
        let lock = self.key_lock(&key);
        let _guard = lock.lock().expect("key lock poisoned");
        self.cache.insert(key, Entry::new(value, ttl));
        async move { Ok(()) }.boxed()
    }

    fn delete(&self, key: String) -> FutureResult<()> {
        tracing::trace!("deleting key: {key} from bucket: {}", self.name);
        let lock = self.key_lock(&key);
        let _guard = lock.lock().expect("key lock poisoned");
        self.cache.invalidate(&key);
        async move { Ok(()) }.boxed()
    }

    fn exists(&self, key: String) -> FutureResult<bool> {
        tracing::trace!("checking existence of key: {key} in bucket: {}", self.name);
        let exists = self.cache.contains_key(&key);
        async move { Ok(exists) }.boxed()
    }

    fn keys(&self) -> FutureResult<Vec<String>> {
        tracing::trace!("listing keys in bucket: {}", self.name);
        let keys = self.cache.iter().map(|(k, _)| (*k).clone()).collect();
        async move { Ok(keys) }.boxed()
    }

    fn increment(&self, key: String, delta: i64, ttl: Option<Duration>) -> FutureResult<i64> {
        tracing::trace!("incrementing key: {key} in bucket: {}", self.name);

        let lock = self.key_lock(&key);
        let result = (|| {
            let _guard = lock.lock().expect("key lock poisoned");
            let existing = self.cache.get(&key);
            let incremented = add_i64(existing.as_ref().map(|entry| entry.bytes.as_slice()), delta)
                .map_err(|err| {
                    Error::Invalid(format!("incrementing `{key}` by {delta}: {err:#}"))
                })?;
            self.cache.insert(key, Entry::atomic(existing, encode_i64(incremented), ttl));
            Ok(incremented)
        })();

        async move { result }.boxed()
    }

    fn compare_and_swap(
        &self, key: String, current: Option<Vec<u8>>, new: Vec<u8>, ttl: Option<Duration>,
    ) -> FutureResult<Result<(), Option<Vec<u8>>>> {
        tracing::trace!("swapping key: {key} in bucket: {}", self.name);

        let lock = self.key_lock(&key);
        let result = {
            let _guard = lock.lock().expect("key lock poisoned");
            let existing = self.cache.get(&key);
            let observed = existing.as_ref().map(|entry| entry.bytes.clone());
            if observed == current {
                self.cache.insert(key, Entry::atomic(existing, new, ttl));
                Ok(())
            } else {
                Err(observed)
            }
        };

        async move { Ok(result) }.boxed()
    }
}

type BucketCache = Cache<String, Entry>;
type KeyLocks = Arc<Mutex<HashMap<String, Weak<Mutex<()>>>>>;

#[derive(Clone)]
struct Entry {
    bytes: Vec<u8>,
    // `None` never expires; a TTL too long for an `Instant` is the same
    expires_at: Option<Instant>,
}

impl Entry {
    fn new(bytes: Vec<u8>, ttl: Option<Duration>) -> Self {
        Self {
            bytes,
            expires_at: ttl.and_then(|ttl| Instant::now().checked_add(ttl)),
        }
    }

    // an atomic applies its TTL only on create; a live entry keeps the time it has left
    fn atomic(existing: Option<Self>, bytes: Vec<u8>, ttl: Option<Duration>) -> Self {
        match existing {
            Some(existing) => Self { bytes, ..existing },
            None => Self::new(bytes, ttl),
        }
    }
}

// expires each entry at its own deadline, on create and on every update
struct Deadline;

impl Expiry<String, Entry> for Deadline {
    fn expire_after_create(&self, _key: &String, entry: &Entry, at: Instant) -> Option<Duration> {
        entry.expires_at.map(|expires_at| expires_at.saturating_duration_since(at))
    }

    fn expire_after_update(
        &self, _key: &String, entry: &Entry, at: Instant, _remaining: Option<Duration>,
    ) -> Option<Duration> {
        entry.expires_at.map(|expires_at| expires_at.saturating_duration_since(at))
    }
}

fn add_i64(current: Option<&[u8]>, delta: i64) -> anyhow::Result<i64> {
    let base = match current {
        None => 0,
        Some(value) => {
            let bytes: [u8; 8] = value.try_into().map_err(|_len| {
                anyhow!("value is {} bytes, not an 8-byte big-endian integer", value.len())
            })?;
            i64::from_be_bytes(bytes)
        }
    };
    base.checked_add(delta).context("adding delta overflows i64")
}

fn encode_i64(value: i64) -> Vec<u8> {
    value.to_be_bytes().to_vec()
}
