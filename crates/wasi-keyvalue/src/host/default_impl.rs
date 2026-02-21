//! Default in-memory implementation for wasi-keyvalue
//!
//! This is a lightweight implementation for development use only.

use std::sync::Arc;

use anyhow::Result;
use futures::FutureExt;
use moka::sync::Cache;
use omnia::Backend;
use tracing::instrument;

use crate::host::WasiKeyValueCtx;
use crate::host::resource::{Bucket, FutureResult};

type BucketCache = Cache<String, Vec<u8>>;

#[derive(Debug, Clone, Default)]
pub struct ConnectOptions;

impl omnia::FromEnv for ConnectOptions {
    fn from_env() -> Result<Self> {
        Ok(Self)
    }
}

/// Default implementation for `wasi:keyvalue`.
#[derive(Clone)]
pub struct KeyValueDefault {
    store: Cache<String, BucketCache>,
}

impl std::fmt::Debug for KeyValueDefault {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("KeyValueDefault").finish_non_exhaustive()
    }
}

impl Backend for KeyValueDefault {
    type ConnectOptions = ConnectOptions;

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

        let cache = self.store.get_with(identifier.clone(), || Cache::builder().build());

        let bucket = InMemBucket {
            name: identifier,
            cache,
        };

        async move { Ok(Arc::new(bucket) as Arc<dyn Bucket>) }.boxed()
    }
}

#[derive(Clone)]
struct InMemBucket {
    name: String,
    cache: BucketCache,
}

impl std::fmt::Debug for InMemBucket {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InMemBucket").field("name", &self.name).finish_non_exhaustive()
    }
}

impl Bucket for InMemBucket {
    fn name(&self) -> &'static str {
        // Note: This returns a static str, but we need to leak the string
        // For a proper implementation, consider changing the trait
        Box::leak(self.name.clone().into_boxed_str())
    }

    fn get(&self, key: String) -> FutureResult<Option<Vec<u8>>> {
        tracing::debug!("getting key: {key} from bucket: {}", self.name);
        let result = self.cache.get(&key);
        async move { Ok(result) }.boxed()
    }

    fn set(&self, key: String, value: Vec<u8>) -> FutureResult<()> {
        tracing::debug!("setting key: {key} in bucket: {}", self.name);
        self.cache.insert(key, value);
        async move { Ok(()) }.boxed()
    }

    fn delete(&self, key: String) -> FutureResult<()> {
        tracing::debug!("deleting key: {key} from bucket: {}", self.name);
        self.cache.invalidate(&key);
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn bucket_operations() {
        let ctx = KeyValueDefault::connect_with(ConnectOptions).await.expect("connect");

        let bucket = ctx.open_bucket("test-bucket".to_string()).await.expect("open bucket");

        // Test set and get
        bucket.set("key1".to_string(), b"value1".to_vec()).await.expect("set");
        let value = bucket.get("key1".to_string()).await.expect("get");
        assert_eq!(value, Some(b"value1".to_vec()));

        // Test exists
        assert!(bucket.exists("key1".to_string()).await.expect("exists"));
        assert!(!bucket.exists("key2".to_string()).await.expect("exists"));

        // Test keys
        bucket.set("key2".to_string(), b"value2".to_vec()).await.expect("set");
        let mut keys = bucket.keys().await.expect("keys");
        keys.sort();
        assert_eq!(keys, vec!["key1".to_string(), "key2".to_string()]);

        // Test delete
        bucket.delete("key1".to_string()).await.expect("delete");
        assert!(!bucket.exists("key1".to_string()).await.expect("exists"));
    }
}
