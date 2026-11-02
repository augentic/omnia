//! Key-value storage service contract and default implementation.

/// The bucket `StateStore` opens.
pub const BUCKET: &str = "cache";

#[cfg(not(target_arch = "wasm32"))]
mod host {
    use std::collections::HashMap;
    use std::fmt::Debug;
    use std::sync::{Arc, Mutex, Weak};
    use std::time::{Duration, Instant};

    use anyhow::{Context as _, Result, anyhow};
    use futures::FutureExt as _;
    use moka::Expiry;
    use moka::sync::Cache;
    use tracing::instrument;

    use crate::{Connect, FutureResult, NoOptions};

    /// Service contract for `wasi:keyvalue` backends.
    pub trait KeyValueClient: Debug + Send + Sync + 'static {
        /// Open a bucket.
        fn open_bucket(&self, identifier: String) -> FutureResult<Arc<dyn Bucket>>;
    }

    /// Stateful handle to a key-value bucket.
    pub trait Bucket: Debug + Send + Sync + 'static {
        /// Get the value associated with `key`.
        fn get(&self, key: String) -> FutureResult<Option<Vec<u8>>>;

        /// Set the value associated with `key`, optionally expiring after `ttl`.
        fn set(&self, key: String, value: Vec<u8>, ttl: Option<Duration>) -> FutureResult<()>;

        /// Delete the value associated with `key`.
        fn delete(&self, key: String) -> FutureResult<()>;

        /// Check whether `key` exists.
        fn exists(&self, key: String) -> FutureResult<bool>;

        /// List all keys in the bucket.
        fn keys(&self) -> FutureResult<Vec<String>>;

        /// Atomically increment the 8-byte big-endian signed integer at `key`
        /// by `delta` (starting at zero when absent).
        fn increment(&self, key: String, delta: i64) -> FutureResult<i64>;

        /// Single-step atomic compare-and-swap: replace the value at `key` with
        /// `value` only when the stored value equals `expected` (`None` means
        /// absent), preserving any remaining TTL. On mismatch returns
        /// `Err(observed)` carrying the value currently stored.
        fn cas(
            &self, key: String, expected: Option<Vec<u8>>, value: Vec<u8>,
        ) -> FutureResult<Result<(), Option<Vec<u8>>>>;
    }

    #[derive(Clone, Debug)]
    struct Entry {
        bytes: Vec<u8>,
        expires_at: Option<Instant>,
    }

    struct EntryExpiry;

    impl Expiry<String, Entry> for EntryExpiry {
        fn expire_after_create(
            &self, _key: &String, value: &Entry, created_at: Instant,
        ) -> Option<Duration> {
            value.expires_at.map(|at| at.saturating_duration_since(created_at))
        }

        fn expire_after_update(
            &self, _key: &String, value: &Entry, updated_at: Instant,
            _duration_until_expiry: Option<Duration>,
        ) -> Option<Duration> {
            value.expires_at.map(|at| at.saturating_duration_since(updated_at))
        }
    }

    type BucketCache = Cache<String, Entry>;
    type KeyLocks = Arc<Mutex<HashMap<String, Weak<Mutex<()>>>>>;

    /// Default in-memory implementation of [`KeyValueClient`].
    #[derive(Clone)]
    pub struct KeyValueDefault {
        store: Cache<String, InMemBucket>,
    }

    impl Debug for KeyValueDefault {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.debug_struct("KeyValueDefault").finish_non_exhaustive()
        }
    }

    impl Connect for KeyValueDefault {
        type ConnectOptions = NoOptions;

        #[instrument]
        async fn connect_with(options: Self::ConnectOptions) -> Result<Self> {
            let _ = options;
            tracing::debug!("initializing in-memory key-value store");
            Ok(Self {
                store: Cache::builder().build(),
            })
        }
    }

    impl KeyValueClient for KeyValueDefault {
        fn open_bucket(&self, identifier: String) -> FutureResult<Arc<dyn Bucket>> {
            tracing::trace!("opening bucket: {identifier}");

            // the lock registry lives with the bucket, so every handle shares its locks
            let bucket = self.store.get_with(identifier.clone(), || InMemBucket {
                name: identifier,
                cache: Cache::builder().expire_after(EntryExpiry).build(),
                locks: Arc::default(),
            });

            async move { Ok(Arc::new(bucket) as Arc<dyn Bucket>) }.boxed()
        }
    }

    #[derive(Clone)]
    struct InMemBucket {
        name: String,
        cache: BucketCache,
        locks: KeyLocks,
    }

    impl Debug for InMemBucket {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.debug_struct("InMemBucket").field("name", &self.name).finish_non_exhaustive()
        }
    }

    impl InMemBucket {
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
            let expires_at = ttl.map(|d| Instant::now() + d);
            self.cache.insert(
                key,
                Entry {
                    bytes: value,
                    expires_at,
                },
            );
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
            let exists = self.cache.get(&key).is_some();
            async move { Ok(exists) }.boxed()
        }

        fn keys(&self) -> FutureResult<Vec<String>> {
            tracing::trace!("listing keys in bucket: {}", self.name);
            let keys = self.cache.iter().map(|(k, _)| (*k).clone()).collect();
            async move { Ok(keys) }.boxed()
        }

        fn increment(&self, key: String, delta: i64) -> FutureResult<i64> {
            tracing::trace!("incrementing key: {key} in bucket: {}", self.name);

            let lock = self.key_lock(&key);
            let result = (|| {
                let _guard = lock.lock().expect("key lock poisoned");
                let existing = self.cache.get(&key);
                let incremented =
                    add_i64(existing.as_ref().map(|entry| entry.bytes.as_slice()), delta)
                        .with_context(|| format!("incrementing `{key}` by {delta}"))?;
                self.cache.insert(
                    key,
                    Entry {
                        bytes: encode_i64(incremented),
                        expires_at: existing.and_then(|entry| entry.expires_at),
                    },
                );
                Ok(incremented)
            })();

            async move { result }.boxed()
        }

        fn cas(
            &self, key: String, expected: Option<Vec<u8>>, value: Vec<u8>,
        ) -> FutureResult<Result<(), Option<Vec<u8>>>> {
            tracing::trace!("cas key: {key} in bucket: {}", self.name);

            let lock = self.key_lock(&key);
            let result = {
                let _guard = lock.lock().expect("key lock poisoned");
                let existing = self.cache.get(&key);
                let observed = existing.as_ref().map(|entry| entry.bytes.clone());
                if observed == expected {
                    self.cache.insert(
                        key,
                        Entry {
                            bytes: value,
                            expires_at: existing.and_then(|entry| entry.expires_at),
                        },
                    );
                    Ok(())
                } else {
                    Err(observed)
                }
            };

            async move { Ok(result) }.boxed()
        }
    }

    fn add_i64(current: Option<&[u8]>, delta: i64) -> Result<i64> {
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

    // TTL expiry and CAS-preserves-TTL are tested here until A8 exposes `omnia:keyvalue/ttl` to guests.
    #[cfg(test)]
    mod tests {
        use std::time::Duration;

        use super::*;

        #[tokio::test]
        async fn ttl_expires_and_cas_preserves_ttl() {
            let store = KeyValueDefault::connect_with(NoOptions).await.expect("connect");
            let bucket = store.open_bucket("b".to_owned()).await.expect("bucket");

            bucket
                .set("ephemeral".to_owned(), b"v1".to_vec(), Some(Duration::from_millis(80)))
                .await
                .expect("set");
            assert_eq!(
                bucket.get("ephemeral".to_owned()).await.expect("get"),
                Some(b"v1".to_vec())
            );

            // CAS updates the bytes and preserves the remaining TTL rather than clearing it.
            assert_eq!(
                bucket
                    .cas("ephemeral".to_owned(), Some(b"v1".to_vec()), b"v2".to_vec())
                    .await
                    .expect("cas"),
                Ok(())
            );
            assert_eq!(
                bucket.get("ephemeral".to_owned()).await.expect("get"),
                Some(b"v2".to_vec())
            );

            std::thread::sleep(Duration::from_millis(120));
            assert_eq!(bucket.get("ephemeral".to_owned()).await.expect("get"), None);
            assert!(!bucket.exists("ephemeral".to_owned()).await.expect("exists"));
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub use self::host::{Bucket, KeyValueClient, KeyValueDefault};
