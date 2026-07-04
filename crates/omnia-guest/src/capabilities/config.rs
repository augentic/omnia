//! Configuration lookup capability.

use std::future::Future;

use anyhow::Result;

/// Provides configuration values from the WASI guest to dependent crates.
pub trait Config: Send + Sync {
    /// Get configuration setting.
    #[cfg(not(target_arch = "wasm32"))]
    fn get(&self, key: &str) -> impl Future<Output = Result<String>> + Send;

    /// Get configuration setting.
    #[cfg(target_arch = "wasm32")]
    fn get(&self, key: &str) -> impl Future<Output = Result<String>> + Send {
        use anyhow::{Context, anyhow};
        async move {
            let config = omnia_wasi_config::store::get(key).context("getting configuration")?;
            config.ok_or_else(|| anyhow!("configuration not found"))
        }
    }
}
