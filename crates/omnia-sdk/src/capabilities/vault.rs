//! Secret vault capability.

use std::future::Future;

use anyhow::Result;

/// Reads and writes secrets in the vault locker (`omnia:vault`).
pub trait Vault: Send + Sync {
    cfg_select! {
        all(target_arch = "wasm32", target_os = "wasi") => {
            /// Retrieve a secret by key from the locker.
            fn get(&self, key: &str) -> impl Future<Output = Result<Option<Vec<u8>>>> + Send {
                use anyhow::Context;
                async move {
                    let locker =
                        omnia_wasi_vault::vault::open(omnia_backend::vault::LOCKER.to_string())
                            .await
                            .context("opening vault locker")?;
                    locker.get(key.to_string()).await.context("getting secret from vault")
                }
            }

            /// Store a secret in the locker.
            fn set(&self, key: &str, value: &[u8]) -> impl Future<Output = Result<()>> + Send {
                use anyhow::Context;
                async move {
                    let locker =
                        omnia_wasi_vault::vault::open(omnia_backend::vault::LOCKER.to_string())
                            .await
                            .context("opening vault locker")?;
                    locker
                        .set(key.to_string(), value.to_vec())
                        .await
                        .context("setting secret in vault")
                }
            }

            /// Delete a secret from the locker.
            fn delete(&self, key: &str) -> impl Future<Output = Result<()>> + Send {
                use anyhow::Context;
                async move {
                    let locker =
                        omnia_wasi_vault::vault::open(omnia_backend::vault::LOCKER.to_string())
                            .await
                            .context("opening vault locker")?;
                    locker.delete(key.to_string()).await.context("deleting secret from vault")
                }
            }

            /// Check whether a secret exists in the locker.
            fn exists(&self, key: &str) -> impl Future<Output = Result<bool>> + Send {
                use anyhow::Context;
                async move {
                    let locker =
                        omnia_wasi_vault::vault::open(omnia_backend::vault::LOCKER.to_string())
                            .await
                            .context("opening vault locker")?;
                    locker.exists(key.to_string()).await.context("checking secret in vault")
                }
            }

            /// List all secret identifiers in the locker.
            fn list_ids(&self) -> impl Future<Output = Result<Vec<String>>> + Send {
                use anyhow::Context;
                async move {
                    let locker =
                        omnia_wasi_vault::vault::open(omnia_backend::vault::LOCKER.to_string())
                            .await
                            .context("opening vault locker")?;
                    locker.list_ids().await.context("listing secrets in vault")
                }
            }
        }
        _ => {
            /// Retrieve a secret by key from the locker.
            fn get(&self, key: &str) -> impl Future<Output = Result<Option<Vec<u8>>>> + Send;

            /// Store a secret in the locker.
            fn set(&self, key: &str, value: &[u8]) -> impl Future<Output = Result<()>> + Send;

            /// Delete a secret from the locker.
            fn delete(&self, key: &str) -> impl Future<Output = Result<()>> + Send;

            /// Check whether a secret exists in the locker.
            fn exists(&self, key: &str) -> impl Future<Output = Result<bool>> + Send;

            /// List all secret identifiers in the locker.
            fn list_ids(&self) -> impl Future<Output = Result<Vec<String>>> + Send;
        }
    }
}

delegate_deref!(Vault {
    fn get(&self, key: &str) -> impl Future<Output = Result<Option<Vec<u8>>>> + Send {
        (**self).get(key)
    }

    fn set(&self, key: &str, value: &[u8]) -> impl Future<Output = Result<()>> + Send {
        (**self).set(key, value)
    }

    fn delete(&self, key: &str) -> impl Future<Output = Result<()>> + Send {
        (**self).delete(key)
    }

    fn exists(&self, key: &str) -> impl Future<Output = Result<bool>> + Send {
        (**self).exists(key)
    }

    fn list_ids(&self) -> impl Future<Output = Result<Vec<String>>> + Send {
        (**self).list_ids()
    }
});
