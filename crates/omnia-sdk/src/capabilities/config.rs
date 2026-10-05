//! Configuration lookup capability.

use std::future::Future;

use anyhow::Result;

/// Provides configuration values from the WASI guest to dependent crates.
pub trait Config: Send + Sync {
    cfg_select! {
        not(target_arch = "wasm32") => {
            /// Get configuration setting.
            fn get(&self, key: &str) -> impl Future<Output = Result<String>> + Send;
        }
        _ => {
            /// Get configuration setting.
            fn get(&self, key: &str) -> impl Future<Output = Result<String>> + Send {
                use anyhow::{Context, anyhow};
                async move {
                    let config =
                        omnia_wasi_config::store::get(key).context("getting configuration")?;
                    config.ok_or_else(|| anyhow!("configuration not found"))
                }
            }
        }
    }
}

delegate_deref!(Config {
    fn get(&self, key: &str) -> impl Future<Output = Result<String>> + Send {
        (**self).get(key)
    }
});
