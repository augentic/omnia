//! Default in-memory implementation for wasi-vault
//!
//! This is a lightweight implementation for development use only.

use std::collections::HashMap;
use std::sync::Arc;

use anyhow::Result;
use futures::FutureExt;
use omnia::Backend;
use tracing::instrument;

use crate::host::WasiVaultCtx;
use crate::host::resource::{FutureResult, Locker};

type Store = Arc<parking_lot::RwLock<HashMap<String, HashMap<String, Vec<u8>>>>>;

/// Options used to connect to the vault.
#[derive(Debug, Clone, Default)]
pub struct ConnectOptions;

impl omnia::FromEnv for ConnectOptions {
    fn from_env() -> Result<Self> {
        Ok(Self)
    }
}

/// Default implementation for `wasi:vault`.
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
        let locker = InMemLocker {
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
struct InMemLocker {
    identifier: String,
    store: Store,
}

impl Locker for InMemLocker {
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
    async fn locker_set_get_delete() {
        let vault = VaultDefault::connect().await.expect("connect");
        let locker = vault.open_locker("app".to_string()).await.expect("open");

        locker.set("api-key".to_string(), b"secret".to_vec()).await.expect("set");
        assert!(locker.exists("api-key".to_string()).await.expect("exists"));
        assert_eq!(locker.get("api-key".to_string()).await.expect("get"), Some(b"secret".to_vec()));
        assert_eq!(locker.list_ids().await.expect("list"), vec!["api-key".to_string()]);

        locker.delete("api-key".to_string()).await.expect("delete");
        assert!(!locker.exists("api-key".to_string()).await.expect("exists"));
        assert_eq!(locker.get("api-key".to_string()).await.expect("get"), None);
    }

    #[tokio::test]
    async fn lockers_are_isolated() {
        let vault = VaultDefault::connect().await.expect("connect");
        let a = vault.open_locker("a".to_string()).await.expect("open a");
        let b = vault.open_locker("b".to_string()).await.expect("open b");

        a.set("k".to_string(), b"a".to_vec()).await.expect("set");
        assert_eq!(b.get("k".to_string()).await.expect("get"), None);
    }
}
