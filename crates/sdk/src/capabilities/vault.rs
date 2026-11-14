//! Secret store capability.

use std::future::Future;

use bytes::Bytes;

use crate::capability::{Page, Result};

/// Read and write secrets in a deployment's lockers.
///
/// A handle reads and writes one locker. A provider's own handle uses
/// [`vault::DEFAULT`](DEFAULT), and [`named`](Self::named) selects another.
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

/// The locker a provider's own [`SecretStore`] handle uses.
pub const DEFAULT: &str = "default";
