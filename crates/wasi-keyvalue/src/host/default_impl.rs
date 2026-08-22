//! Default in-memory implementation for wasi-keyvalue
//!
//! This is a lightweight implementation for development use only.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use anyhow::{Context as _, Result, anyhow};
use futures::FutureExt;
use moka::sync::Cache;
use omnia::Backend;
use tracing::instrument;

use crate::host::WasiKeyValueCtx;
use crate::host::resource::{Bucket, Cas, FutureResult};

type BucketCache = Cache<String, Vec<u8>>;
type KeyLocks = Arc<Mutex<HashMap<String, Arc<Mutex<()>>>>>;

/// Default implementation for `wasi:keyvalue`.
#[derive(Clone)]
pub struct KeyValueDefault {
    store: Cache<String, InMemBucket>,
}

impl std::fmt::Debug for KeyValueDefault {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("KeyValueDefault").finish_non_exhaustive()
    }
}

impl Backend for KeyValueDefault {
    type ConnectOptions = omnia::NoOptions;

    #[instrument]
    async fn connect_with(options: Self::ConnectOptions) -> Result<Self> {
        tracing::debug!("initializing in-memory key-value store");
        Ok(Self {
            store: Cache::builder().build(),
        })
    }
}

impl WasiKeyValueCtx for KeyValueDefault {
    fn open_bucket(&self, identifier: String) -> FutureResult<Arc<dyn Bucket>> {
        tracing::debug!("opening bucket: {identifier}");

        // The lock registry lives with the bucket identity so every handle to
        // the same bucket serializes writes and atomics on the same locks.
        let bucket = self.store.get_with(identifier.clone(), || InMemBucket {
            name: identifier,
            cache: Cache::builder().build(),
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

impl std::fmt::Debug for InMemBucket {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InMemBucket").field("name", &self.name).finish_non_exhaustive()
    }
}

impl InMemBucket {
    /// Per-key mutex so writes and atomics serialize per key, not per bucket.
    fn key_lock(&self, key: &str) -> Arc<Mutex<()>> {
        let mut locks = self.locks.lock().expect("lock registry poisoned");
        Arc::clone(locks.entry(key.to_string()).or_default())
    }
}

impl Bucket for InMemBucket {
    fn get(&self, key: String) -> FutureResult<Option<Vec<u8>>> {
        tracing::debug!("getting key: {key} from bucket: {}", self.name);
        let result = self.cache.get(&key);
        async move { Ok(result) }.boxed()
    }

    fn set(&self, key: String, value: Vec<u8>) -> FutureResult<()> {
        tracing::debug!("setting key: {key} in bucket: {}", self.name);
        let lock = self.key_lock(&key);
        let _guard = lock.lock().expect("key lock poisoned");
        self.cache.insert(key, value);
        async move { Ok(()) }.boxed()
    }

    fn delete(&self, key: String) -> FutureResult<()> {
        tracing::debug!("deleting key: {key} from bucket: {}", self.name);
        let lock = self.key_lock(&key);
        let _guard = lock.lock().expect("key lock poisoned");
        self.cache.invalidate(&key);
        self.locks.lock().expect("lock registry poisoned").remove(&key);
        async move { Ok(()) }.boxed()
    }

    fn exists(&self, key: String) -> FutureResult<bool> {
        tracing::debug!("checking existence of key: {key} in bucket: {}", self.name);
        let exists = self.cache.contains_key(&key);
        async move { Ok(exists) }.boxed()
    }

    fn keys(&self) -> FutureResult<Vec<String>> {
        tracing::debug!("listing keys in bucket: {}", self.name);
        let keys = self.cache.iter().map(|(k, _)| (*k).clone()).collect();
        async move { Ok(keys) }.boxed()
    }

    fn increment(&self, key: String, delta: i64) -> FutureResult<i64> {
        tracing::debug!("incrementing key: {key} in bucket: {}", self.name);

        let lock = self.key_lock(&key);
        let result = (|| {
            let _guard = lock.lock().expect("key lock poisoned");
            let incremented = add_i64(self.cache.get(&key).as_deref(), delta)
                .with_context(|| format!("incrementing `{key}` by {delta}"))?;
            self.cache.insert(key, encode_i64(incremented));
            Ok(incremented)
        })();

        async move { result }.boxed()
    }

    fn swap(&self, cas: Cas, value: Vec<u8>) -> FutureResult<Result<(), Cas>> {
        tracing::debug!("swapping key: {} in bucket: {}", cas.key, self.name);

        let lock = self.key_lock(&cas.key);
        let result = {
            let _guard = lock.lock().expect("key lock poisoned");
            let observed = self.cache.get(&cas.key);
            if observed == cas.current {
                self.cache.insert(cas.key, value);
                Ok(())
            } else {
                Err(Cas {
                    current: observed,
                    ..cas
                })
            }
        };

        async move { Ok(result) }.boxed()
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
