//! Default in-memory implementation for wasi-vault
//!
//! This is a lightweight implementation for development use only.

use std::collections::HashMap;
use std::sync::Arc;

use anyhow::Result;
use futures::FutureExt;
use tracing::instrument;
use yetti::Backend;

use crate::host::WasiVaultCtx;
use crate::host::resource::{FutureResult, Locker};

type Store = Arc<parking_lot::RwLock<HashMap<String, HashMap<String, Vec<u8>>>>>;

#[derive(Debug, Clone, Default)]
pub struct ConnectOptions;

impl yetti::FromEnv for ConnectOptions {
    fn from_env() -> Result<Self> {
        Ok(Self)
    }
}

#[derive(Debug, Clone)]
pub struct VaultDefault {
    // Using Arc for shared state across instances
    store: Store,
}

impl Backend for VaultDefault {
    type ConnectOptions = ConnectOptions;

    #[instrument]
    async fn connect_with(options: Self::ConnectOptions) -> Result<Self> {
        tracing::debug!("initializing in-memory vault");
        Ok(Self {
            store: Arc::new(parking_lot::RwLock::new(HashMap::new())),
        })
    }
}

impl WasiVaultCtx for VaultDefault {
    fn open_locker(&self, identifier: String) -> FutureResult<Arc<dyn Locker>> {
        tracing::debug!("opening locker: {}", identifier);
        let locker = InMemoryLocker {
            identifier: identifier.clone(),
            store: Arc::clone(&self.store),
        };

        // Ensure locker exists in store
        {
            let mut store = self.store.write();
            store.entry(identifier).or_default()
        };

        async move { Ok(Arc::new(locker) as Arc<dyn Locker>) }.boxed()
    }
}

#[derive(Debug, Clone)]
struct InMemoryLocker {
    identifier: String,
    store: Store,
}

impl Locker for InMemoryLocker {
    fn identifier(&self) -> String {
        self.identifier.clone()
    }

    fn get(&self, secret_id: String) -> FutureResult<Option<Vec<u8>>> {
        tracing::debug!("getting secret: {} from locker: {}", secret_id, self.identifier);
        let store = Arc::clone(&self.store);
        let locker_id = self.identifier.clone();

        async move {
            let result = {
                let store = store.read();
                store.get(&locker_id).and_then(|locker| locker.get(&secret_id).cloned())
            };
            Ok(result)
        }
        .boxed()
    }

    fn set(&self, secret_id: String, value: Vec<u8>) -> FutureResult<()> {
        tracing::debug!("setting secret: {} in locker: {}", secret_id, self.identifier);
        let store = Arc::clone(&self.store);
        let locker_id = self.identifier.clone();

        async move {
            {
                let mut store = store.write();
                store.entry(locker_id).or_default().insert(secret_id, value)
            };
            Ok(())
        }
        .boxed()
    }

    fn delete(&self, secret_id: String) -> FutureResult<()> {
        tracing::debug!("deleting secret: {} from locker: {}", secret_id, self.identifier);
        let store = Arc::clone(&self.store);
        let locker_id = self.identifier.clone();

        async move {
            {
                let mut store = store.write();
                if let Some(locker) = store.get_mut(&locker_id) {
                    locker.remove(&secret_id);
                }
            }
            Ok(())
        }
        .boxed()
    }

    fn exists(&self, secret_id: String) -> FutureResult<bool> {
        tracing::debug!(
            "checking existence of secret: {} in locker: {}",
            secret_id,
            self.identifier
        );
        let store = Arc::clone(&self.store);
        let locker_id = self.identifier.clone();

        async move {
            let exists = {
                let store = store.read();
                store.get(&locker_id).is_some_and(|locker| locker.contains_key(&secret_id))
            };
            Ok(exists)
        }
        .boxed()
    }

    fn list_ids(&self) -> FutureResult<Vec<String>> {
        tracing::debug!("listing secrets in locker: {}", self.identifier);
        let store = Arc::clone(&self.store);
        let locker_id = self.identifier.clone();

        async move {
            let ids = {
                let store = store.read();
                store
                    .get(&locker_id)
                    .map(|locker| locker.keys().cloned().collect())
                    .unwrap_or_default()
            };
            Ok(ids)
        }
        .boxed()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn locker_operations() {
        let ctx = VaultDefault::connect_with(ConnectOptions).await.expect("connect");

        let locker = ctx.open_locker("test-locker".to_string()).await.expect("open locker");

        // Test set and get
        locker.set("secret1".to_string(), b"value1".to_vec()).await.expect("set");
        let value = locker.get("secret1".to_string()).await.expect("get");
        assert_eq!(value, Some(b"value1".to_vec()));

        // Test exists
        assert!(locker.exists("secret1".to_string()).await.expect("exists"));
        assert!(!locker.exists("secret2".to_string()).await.expect("exists"));

        // Test list_ids
        locker.set("secret2".to_string(), b"value2".to_vec()).await.expect("set");
        let mut ids = locker.list_ids().await.expect("list_ids");
        ids.sort();
        assert_eq!(ids, vec!["secret1".to_string(), "secret2".to_string()]);

        // Test delete
        locker.delete("secret1".to_string()).await.expect("delete");
        assert!(!locker.exists("secret1".to_string()).await.expect("exists"));

        // Test identifier
        assert_eq!(locker.identifier(), "test-locker");
    }
}
