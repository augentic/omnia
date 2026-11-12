//! Configuration lookup capability.

use std::fmt;
use std::future::Future;
use std::str::FromStr;

use anyhow::Result;

/// Provides configuration values from the WASI guest to dependent crates.
///
/// Typed reads over the string values are [`ConfigExt`].
#[omnia_guest_macros::capability]
pub trait Config: Send + Sync {
    /// Get configuration setting.
    fn get(&self, key: &str) -> impl Future<Output = Result<String>> + Send;
}

/// Typed reads over any [`Config`].
///
/// Blanket-implemented for every [`Config`]; never implemented by hand.
pub trait ConfigExt: Config {
    /// Get a configuration setting and parse it into `T`.
    ///
    /// Fails when `key` is missing or its value does not parse as `T`.
    fn get_parsed<T>(&self, key: &str) -> impl Future<Output = Result<T>> + Send
    where
        T: FromStr,
        T::Err: fmt::Display,
    {
        async move {
            let value = self.get(key).await?;
            value.parse::<T>().map_err(|error| anyhow::anyhow!("`{key}` is invalid: {error}"))
        }
    }
}

impl<P: Config + ?Sized> ConfigExt for P {}

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
