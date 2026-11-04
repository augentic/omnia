//! Configuration lookup capability.

use std::future::Future;

use anyhow::Result;

/// Provides configuration values from the WASI guest to dependent crates.
#[omnia_guest_macros::capability]
pub trait Config: Send + Sync {
    /// Get configuration setting.
    fn get(&self, key: &str) -> impl Future<Output = Result<String>> + Send;
}

#[cfg(all(target_arch = "wasm32", target_os = "wasi"))]
impl Config for crate::WasiProvider {
    fn get(&self, key: &str) -> impl Future<Output = Result<String>> + Send {
        use anyhow::{Context, anyhow};

        std::future::ready(
            omnia_wasi_config::store::get(key)
                .context("getting configuration")
                .and_then(|config| config.ok_or_else(|| anyhow!("configuration not found"))),
        )
    }
}
