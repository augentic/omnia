//! Default in-memory implementation for `omnia:vault`
//!
//! This is a lightweight implementation for development use only.

use std::collections::HashMap;
use std::sync::Arc;

use anyhow::Result;
use futures::FutureExt;
use omnia_core::{Connect, FromEnv};
use parking_lot::RwLock;
use tracing::instrument;

use crate::resource::{FutureResult, Locker};
use crate::{Error, VaultClient};

/// Options used to connect to the in-memory vault.
///
/// This struct is used to load connection options from environment variables.
#[derive(Debug, Clone, FromEnv)]
pub struct ConnectOptions {
    /// The lockers a guest may open (`VAULT_LOCKERS`, comma-separated, default: `default`).
    #[env(from = "VAULT_LOCKERS", default = "default", with = omnia_core::env::list)]
    pub lockers: Vec<String>,
}

/// Default implementation for `omnia:vault`.
///
/// It serves the lockers its [`ConnectOptions`] declares, and refuses every
/// other name with [`Error::NotFound`].
#[derive(Debug, Clone)]
pub struct VaultDefault {
    store: Store,
}

impl Connect for VaultDefault {
    type ConnectOptions = ConnectOptions;

    #[instrument]
    async fn connect_with(options: Self::ConnectOptions) -> Result<Self> {
        tracing::debug!("initializing in-memory vault");
        let lockers = options.lockers.into_iter().map(|name| (name, HashMap::new())).collect();
        Ok(Self {
            store: Arc::new(RwLock::new(lockers)),
        })
    }
}

impl VaultClient for VaultDefault {
    fn open_locker(&self, identifier: String) -> FutureResult<Arc<dyn Locker>> {
        tracing::trace!("opening locker: {identifier}");
        let declared = self.store.read().contains_key(&identifier);
        let store = Arc::clone(&self.store);
        async move {
            anyhow::ensure!(
                declared,
                Error::NotFound(format!("locker `{identifier}` is not declared"))
            );
            Ok(Arc::new(InMemLocker { identifier, store }) as Arc<dyn Locker>)
        }
        .boxed()
    }
}

type Store = Arc<RwLock<HashMap<String, HashMap<String, Vec<u8>>>>>;

#[derive(Debug, Clone)]
struct InMemLocker {
    identifier: String,
    store: Store,
}

impl Locker for InMemLocker {
    fn get(&self, name: String) -> FutureResult<Option<Vec<u8>>> {
        tracing::trace!("getting secret: {name} from locker: {}", self.identifier);
        let store = Arc::clone(&self.store);
        let locker_id = self.identifier.clone();

        async move {
            let result = {
                let store = store.read();
                store.get(&locker_id).and_then(|locker| locker.get(&name).cloned())
            };
            Ok(result)
        }
        .boxed()
    }

    fn set(&self, name: String, value: Vec<u8>) -> FutureResult<()> {
        tracing::trace!("setting secret: {name} in locker: {}", self.identifier);
        let store = Arc::clone(&self.store);
        let locker_id = self.identifier.clone();

        async move {
            {
                let mut store = store.write();
                store.entry(locker_id).or_default().insert(name, value)
            };
            Ok(())
        }
        .boxed()
    }

    fn delete(&self, name: String) -> FutureResult<()> {
        tracing::trace!("deleting secret: {name} from locker: {}", self.identifier);
        let store = Arc::clone(&self.store);
        let locker_id = self.identifier.clone();

        async move {
            {
                let mut store = store.write();
                if let Some(locker) = store.get_mut(&locker_id) {
                    locker.remove(&name);
                }
            }
            Ok(())
        }
        .boxed()
    }

    fn exists(&self, name: String) -> FutureResult<bool> {
        tracing::trace!("checking existence of secret: {name} in locker: {}", self.identifier);
        let store = Arc::clone(&self.store);
        let locker_id = self.identifier.clone();

        async move {
            let exists = {
                let store = store.read();
                store.get(&locker_id).is_some_and(|locker| locker.contains_key(&name))
            };
            Ok(exists)
        }
        .boxed()
    }

    fn list_ids(&self) -> FutureResult<Vec<String>> {
        tracing::trace!("listing secrets in locker: {}", self.identifier);
        let store = Arc::clone(&self.store);
        let locker_id = self.identifier.clone();

        async move {
            let names = {
                let store = store.read();
                store.get(&locker_id).map_or_default(|locker| locker.keys().cloned().collect())
            };
            Ok(names)
        }
        .boxed()
    }
}
