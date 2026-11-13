//! Secret store capability.

use std::future::Future;

use bytes::Bytes;

use crate::capability::{Page, Result};
#[cfg(all(target_arch = "wasm32", target_os = "wasi"))]
use crate::vault::{DEFAULT, Handle};

/// Read and write secrets in a deployment's lockers.
///
/// A handle reads and writes one locker. A provider's own handle uses
/// [`vault::DEFAULT`](crate::vault::DEFAULT), and [`named`](Self::named)
/// selects another.
#[omnia_guest_macros::capability]
pub trait SecretStore: Send + Sync {
    /// The handle [`named`](Self::named) returns.
    type Named: SecretStore<Named = Self::Named>;

    /// Retrieve the secret at `key`, or `None` when it is absent.
    fn get(&self, key: &str) -> impl Future<Output = Result<Option<Bytes>>> + Send;

    /// Store `value` at `key`, replacing any secret there.
    fn set(&self, key: &str, value: &[u8]) -> impl Future<Output = Result<()>> + Send;

    /// Delete the secret at `key`, succeeding when it is already absent.
    fn delete(&self, key: &str) -> impl Future<Output = Result<()>> + Send;

    /// One page of the keys in this locker.
    ///
    /// `after` is the [`next`](Page::next) cursor of the previous page, or
    /// `None` for the first page.
    fn keys(&self, after: Option<&str>) -> impl Future<Output = Result<Page<String>>> + Send;

    /// A handle to the locker `name` on the same provider.
    fn named(&self, name: &str) -> Self::Named;

    /// Whether a secret is stored at `key`.
    fn exists(&self, key: &str) -> impl Future<Output = Result<bool>> + Send {
        async move { Ok(self.get(key).await?.is_some()) }
    }
}

#[cfg(all(target_arch = "wasm32", target_os = "wasi"))]
impl SecretStore for crate::WasiProvider {
    type Named = Handle;

    async fn get(&self, key: &str) -> Result<Option<Bytes>> {
        Handle::new(DEFAULT).get(key).await
    }

    async fn set(&self, key: &str, value: &[u8]) -> Result<()> {
        Handle::new(DEFAULT).set(key, value).await
    }

    async fn delete(&self, key: &str) -> Result<()> {
        Handle::new(DEFAULT).delete(key).await
    }

    async fn keys(&self, after: Option<&str>) -> Result<Page<String>> {
        Handle::new(DEFAULT).keys(after).await
    }

    fn named(&self, name: &str) -> Handle {
        Handle::new(name)
    }

    async fn exists(&self, key: &str) -> Result<bool> {
        Handle::new(DEFAULT).exists(key).await
    }
}
