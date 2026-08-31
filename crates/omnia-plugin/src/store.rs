//! Persistence behind [`RegistryClient`](crate::RegistryClient):
//! content-addressed bytes and per-registry release records.
//!
//! The store is a byte cache and offline fallback, never an authority: the
//! acquirer resolves releases fresh whenever the registry is reachable and
//! verifies stored content against the fresh digest before serving it.
//! Content entries are digest-keyed and shared across registries; release
//! records are scoped per registry so an endpoint override is never answered
//! from another registry's record.

use anyhow::Result;
use futures::FutureExt as _;
use futures::future::BoxFuture;

/// Both halves of the persistence behind [`RegistryClient`](crate::RegistryClient).
pub trait PluginStore: ContentStore + ReleaseStore {}

impl<T: ContentStore + ReleaseStore> PluginStore for T {}

/// Content-addressed persistence: digest-keyed bytes shared across registries.
pub trait ContentStore: Send + Sync + 'static {
    /// The stored bytes keyed by `digest`, if any.
    ///
    /// # Errors
    ///
    /// Returns an error if the store cannot be read.
    fn content<'a>(&'a self, digest: &'a str) -> BoxFuture<'a, Result<Option<Vec<u8>>>>;

    /// Persist `bytes` under `digest`, refusing bytes that do not hash to it.
    ///
    /// # Errors
    ///
    /// Returns an error if the bytes do not hash to `digest`, or if the
    /// store cannot persist them.
    fn put_content<'a>(&'a self, digest: &'a str, bytes: &'a [u8]) -> BoxFuture<'a, Result<()>>;
}

/// Per-registry resolution index: an exact package version to its digest.
pub trait ReleaseStore: Send + Sync + 'static {
    /// The stored record of `package` at `version` in `registry`.
    ///
    /// # Errors
    ///
    /// Returns an error if the store cannot be read.
    fn release<'a>(
        &'a self, registry: &'a str, package: &'a str, version: &'a str,
    ) -> BoxFuture<'a, Result<Option<ReleaseRecord>>>;

    /// Persist `record` as the resolution of `package` in `registry`.
    ///
    /// # Errors
    ///
    /// Returns an error if the store cannot persist the record.
    fn put_release<'a>(
        &'a self, registry: &'a str, package: &'a str, record: &'a ReleaseRecord,
    ) -> BoxFuture<'a, Result<()>>;
}

/// One stored release resolution.
#[derive(Clone, Debug, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
pub struct ReleaseRecord {
    /// Exact semver version of the release.
    pub version: String,
    /// The registry's `sha256:<hex>` digest for the release content.
    pub content_digest: String,
}

/// The cacheless [`PluginStore`], the default for
/// [`RegistryClient`](crate::RegistryClient).
#[derive(Clone, Copy, Debug, Default)]
pub struct NoStore;

impl ContentStore for NoStore {
    fn content<'a>(&'a self, _digest: &'a str) -> BoxFuture<'a, Result<Option<Vec<u8>>>> {
        async { Ok(None) }.boxed()
    }

    fn put_content<'a>(&'a self, _digest: &'a str, _bytes: &'a [u8]) -> BoxFuture<'a, Result<()>> {
        async { Ok(()) }.boxed()
    }
}

impl ReleaseStore for NoStore {
    fn release<'a>(
        &'a self, _registry: &'a str, _package: &'a str, _version: &'a str,
    ) -> BoxFuture<'a, Result<Option<ReleaseRecord>>> {
        async { Ok(None) }.boxed()
    }

    fn put_release<'a>(
        &'a self, _registry: &'a str, _package: &'a str, _record: &'a ReleaseRecord,
    ) -> BoxFuture<'a, Result<()>> {
        async { Ok(()) }.boxed()
    }
}
