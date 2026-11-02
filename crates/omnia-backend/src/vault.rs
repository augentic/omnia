//! Secret vault service contract, constants, and default implementation.

/// The locker the `Vault` capability opens.
pub const LOCKER: &str = "locker";

#[cfg(not(target_arch = "wasm32"))]
mod host {
    use std::collections::HashMap;
    use std::fmt::Debug;
    use std::sync::Arc;

    use anyhow::Result;
    use futures::FutureExt as _;
    use tracing::instrument;

    use crate::{Connect, FutureResult, NoOptions};

    /// Service contract for `omnia:vault` backends.
    pub trait VaultClient: Debug + Send + Sync + 'static {
        /// Open a locker by identifier.
        fn open_locker(&self, identifier: String) -> FutureResult<Arc<dyn Locker>>;
    }

    /// Stateful handle to a secret locker.
    pub trait Locker: Debug + Send + Sync + 'static {
        /// Get the secret value associated with `secret_id`.
        fn get(&self, secret_id: String) -> FutureResult<Option<Vec<u8>>>;

        /// Store `value` under `secret_id`.
        fn set(&self, secret_id: String, value: Vec<u8>) -> FutureResult<()>;

        /// Delete `secret_id`.
        fn delete(&self, secret_id: String) -> FutureResult<()>;

        /// Check whether `secret_id` exists.
        fn exists(&self, secret_id: String) -> FutureResult<bool>;

        /// List all secret IDs in the locker.
        fn list_ids(&self) -> FutureResult<Vec<String>>;
    }

    type Store = Arc<parking_lot::RwLock<HashMap<String, HashMap<String, Vec<u8>>>>>;

    /// Default in-memory implementation of [`VaultClient`].
    #[derive(Debug, Clone, Default)]
    pub struct VaultDefault {
        store: Store,
    }

    impl Connect for VaultDefault {
        type ConnectOptions = NoOptions;

        #[instrument]
        async fn connect_with(options: Self::ConnectOptions) -> Result<Self> {
            let _ = options;
            tracing::debug!("initializing in-memory vault");
            Ok(Self::default())
        }
    }

    impl VaultClient for VaultDefault {
        fn open_locker(&self, identifier: String) -> FutureResult<Arc<dyn Locker>> {
            tracing::trace!("opening locker: {identifier}");
            let locker = InMemLocker {
                identifier: identifier.clone(),
                store: Arc::clone(&self.store),
            };

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
        fn get(&self, secret_id: String) -> FutureResult<Option<Vec<u8>>> {
            tracing::trace!("getting secret: {} from locker: {}", secret_id, self.identifier);
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
            tracing::trace!("setting secret: {} in locker: {}", secret_id, self.identifier);
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
            tracing::trace!("deleting secret: {} from locker: {}", secret_id, self.identifier);
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
            tracing::trace!(
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
            tracing::trace!("listing secrets in locker: {}", self.identifier);
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
}

#[cfg(not(target_arch = "wasm32"))]
pub use self::host::{Locker, VaultClient, VaultDefault};
