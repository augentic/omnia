//! Default in-memory implementation for wasi-keyvalue
//!
//! This is a lightweight implementation for development use only.

use std::collections::HashMap;
use std::sync::Arc;

use anyhow::Result;
use dashmap::DashMap;
use futures::FutureExt;
use qwasr::Backend;
use tracing::instrument;

use crate::host::WasiKeyValueCtx;
use crate::host::resource::{Bucket, FutureResult};

type Store = Arc<DashMap<String, HashMap<String, Vec<u8>>>>;

#[derive(Debug, Clone, Default)]
pub struct ConnectOptions;

impl qwasr::FromEnv for ConnectOptions {
    fn from_env() -> Result<Self> {
        Ok(Self)
    }
}

/// Default implementation for `wasi:keyvalue`.
#[derive(Debug, Clone)]
pub struct KeyValueDefault {
    store: Store,
}

impl Backend for KeyValueDefault {
    type ConnectOptions = ConnectOptions;

    #[instrument]
    async fn connect_with(options: Self::ConnectOptions) -> Result<Self> {
        tracing::debug!("initializing in-memory key-value store");
        Ok(Self {
            store: Arc::new(DashMap::new()),
        })
    }
}

impl WasiKeyValueCtx for KeyValueDefault {
    fn open_bucket(&self, identifier: String) -> FutureResult<Arc<dyn Bucket>> {
        tracing::debug!("opening bucket: {identifier}");

        self.store.entry(identifier.clone()).or_default();

        let bucket = InMemBucket {
            name: identifier,
            store: Arc::clone(&self.store),
        };

        async move { Ok(Arc::new(bucket) as Arc<dyn Bucket>) }.boxed()
    }
}

#[derive(Debug, Clone)]
struct InMemBucket {
    name: String,
    store: Store,
}

impl Bucket for InMemBucket {
    fn name(&self) -> &'static str {
        // Note: This returns a static str, but we need to leak the string
        // For a proper implementation, consider changing the trait
        Box::leak(self.name.clone().into_boxed_str())
    }

    fn get(&self, key: String) -> FutureResult<Option<Vec<u8>>> {
        tracing::debug!("getting key: {key} from bucket: {}", self.name);
        let result = self.store.get(&self.name).and_then(|bucket| bucket.get(&key).cloned());
        async move { Ok(result) }.boxed()
    }

    fn set(&self, key: String, value: Vec<u8>) -> FutureResult<()> {
        tracing::debug!("setting key: {key} in bucket: {}", self.name);
        self.store.entry(self.name.clone()).or_default().insert(key, value);
        async move { Ok(()) }.boxed()
    }

    fn delete(&self, key: String) -> FutureResult<()> {
        tracing::debug!("deleting key: {key} from bucket: {}", self.name);
        if let Some(mut bucket) = self.store.get_mut(&self.name) {
            bucket.remove(&key);
        }
        async move { Ok(()) }.boxed()
    }

    fn exists(&self, key: String) -> FutureResult<bool> {
        tracing::debug!("checking existence of key: {key} in bucket: {}", self.name);
        let exists = self.store.get(&self.name).is_some_and(|bucket| bucket.contains_key(&key));
        async move { Ok(exists) }.boxed()
    }

    fn keys(&self) -> FutureResult<Vec<String>> {
        tracing::debug!("listing keys in bucket: {}", self.name);
        let keys = self
            .store
            .get(&self.name)
            .map(|bucket| bucket.keys().cloned().collect())
            .unwrap_or_default();
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
