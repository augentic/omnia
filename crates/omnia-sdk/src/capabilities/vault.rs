//! Secret vault capability.

use std::future::Future;

use anyhow::Result;

/// Reads and writes secrets in the vault locker (`omnia:vault`).
#[omnia_guest_macros::capability]
pub trait Vault: Send + Sync {
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

#[cfg(all(target_arch = "wasm32", target_os = "wasi"))]
impl Vault for crate::WasiProvider {
    async fn get(&self, key: &str) -> Result<Option<Vec<u8>>> {
        use anyhow::Context;

        let locker = omnia_wasi_vault::vault::open(crate::vault::LOCKER.to_string())
            .await
            .context("opening vault locker")?;
        locker.get(key.to_string()).await.context("getting secret from vault")
    }

    async fn set(&self, key: &str, value: &[u8]) -> Result<()> {
        use anyhow::Context;

        let locker = omnia_wasi_vault::vault::open(crate::vault::LOCKER.to_string())
            .await
            .context("opening vault locker")?;
        locker.set(key.to_string(), value.to_vec()).await.context("setting secret in vault")
    }

    async fn delete(&self, key: &str) -> Result<()> {
        use anyhow::Context;

        let locker = omnia_wasi_vault::vault::open(crate::vault::LOCKER.to_string())
            .await
            .context("opening vault locker")?;
        locker.delete(key.to_string()).await.context("deleting secret from vault")
    }

    async fn exists(&self, key: &str) -> Result<bool> {
        use anyhow::Context;

        let locker = omnia_wasi_vault::vault::open(crate::vault::LOCKER.to_string())
            .await
            .context("opening vault locker")?;
        locker.exists(key.to_string()).await.context("checking secret in vault")
    }

    async fn list_ids(&self) -> Result<Vec<String>> {
        use anyhow::Context;

        let locker = omnia_wasi_vault::vault::open(crate::vault::LOCKER.to_string())
            .await
            .context("opening vault locker")?;
        locker.list_ids().await.context("listing secrets in vault")
    }
}
