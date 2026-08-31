//! Persistence behind [`RegistryAcquire`](crate::RegistryAcquire):
//! content-addressed bytes and per-registry release records.

use std::fmt::Write as _;

use anyhow::Result;
use futures::FutureExt as _;
use futures::future::BoxFuture;
use sha2::{Digest as _, Sha256};

/// One stored release resolution: the exact version plus the registry's
/// content digest for it.
#[derive(Clone, Debug, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
pub struct ReleaseRecord {
    /// Exact semver version of the release.
    pub version: String,
    /// The registry's `sha256:<hex>` digest for the release content.
    pub content_digest: String,
}

/// Content-addressed persistence: digest-keyed bytes shared across registries.
///
/// The digest is the identity. Bytes that do not hash to their key must be
/// refused — a store entry is trusted by its name.
pub trait ContentStore: Send + Sync + 'static {
    /// The stored bytes keyed by `digest`, if any.
    ///
    /// # Errors
    ///
    /// Returns an error if the store cannot be read.
    fn get<'a>(&'a self, digest: &'a str) -> BoxFuture<'a, Result<Option<Vec<u8>>>>;

    /// Persist `bytes` under `digest`.
    ///
    /// # Errors
    ///
    /// Returns an error if the bytes do not hash to `digest`, or if the
    /// store cannot persist them.
    fn put<'a>(&'a self, digest: &'a str, bytes: &'a [u8]) -> BoxFuture<'a, Result<()>>;
}

/// Per-registry resolution index: an exact package version to its digest.
///
/// Records are scoped per registry so an endpoint override is never
/// answered from another registry's record.
pub trait ReleaseStore: Send + Sync + 'static {
    /// The stored record of `package` at `version` in `registry`.
    ///
    /// # Errors
    ///
    /// Returns an error if the store cannot be read.
    fn get<'a>(
        &'a self, registry: &'a str, package: &'a str, version: &'a str,
    ) -> BoxFuture<'a, Result<Option<ReleaseRecord>>>;

    /// Persist `record` as the resolution of `package` in `registry`.
    ///
    /// # Errors
    ///
    /// Returns an error if the store cannot persist the record.
    fn put<'a>(
        &'a self, registry: &'a str, package: &'a str, record: &'a ReleaseRecord,
    ) -> BoxFuture<'a, Result<()>>;
}

/// Both halves of the persistence behind [`RegistryAcquire`](crate::RegistryAcquire).
///
/// The store is a fallback and a byte cache, never an authority: the acquirer
/// resolves releases fresh whenever the registry is reachable, refreshing the
/// stored record, and verifies stored content against the fresh digest before
/// serving it.
pub trait PluginStore: ContentStore + ReleaseStore {}

impl<T: ContentStore + ReleaseStore> PluginStore for T {}

/// The cacheless [`ContentStore`] and [`ReleaseStore`].
///
/// Stores nothing, remembers nothing: every load resolves and fetches fresh,
/// and registry unavailability has no fallback. The default for
/// [`RegistryAcquire`](crate::RegistryAcquire).
#[derive(Clone, Copy, Debug, Default)]
pub struct NoStore;

impl ContentStore for NoStore {
    fn get<'a>(&'a self, _digest: &'a str) -> BoxFuture<'a, Result<Option<Vec<u8>>>> {
        async { Ok(None) }.boxed()
    }

    fn put<'a>(&'a self, _digest: &'a str, _bytes: &'a [u8]) -> BoxFuture<'a, Result<()>> {
        async { Ok(()) }.boxed()
    }
}

impl ReleaseStore for NoStore {
    fn get<'a>(
        &'a self, _registry: &'a str, _package: &'a str, _version: &'a str,
    ) -> BoxFuture<'a, Result<Option<ReleaseRecord>>> {
        async { Ok(None) }.boxed()
    }

    fn put<'a>(
        &'a self, _registry: &'a str, _package: &'a str, _record: &'a ReleaseRecord,
    ) -> BoxFuture<'a, Result<()>> {
        async { Ok(()) }.boxed()
    }
}

/// Hash `bytes` into their canonical `sha256:<hex>` digest string.
#[must_use]
pub fn sha256_digest(bytes: &[u8]) -> String {
    let hash = Sha256::digest(bytes);
    let mut digest = String::with_capacity("sha256:".len() + 2 * hash.len());
    digest.push_str("sha256:");
    for byte in hash {
        let _ = write!(digest, "{byte:02x}");
    }
    digest
}
