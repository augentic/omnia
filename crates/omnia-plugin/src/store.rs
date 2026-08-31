//! Digest-keyed persistence for registry-acquired plugin content.

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

/// Digest-keyed persistence behind [`RegistryAcquire`](crate::RegistryAcquire).
///
/// Content entries are keyed by `sha256:<hex>` digest and shared across
/// registries — the digest is the identity. Release records map an exact
/// package version to its digest and are scoped per registry, so an
/// endpoint override is never answered from another registry's record.
///
/// The store is a fallback and a byte cache, never an authority: the acquirer
/// resolves releases fresh whenever the registry is reachable, refreshing the
/// stored record, and verifies stored content against the fresh digest before
/// serving it.
pub trait PluginStore: Send + Sync + 'static {
    /// The stored bytes keyed by `digest`, if any.
    fn get_content<'a>(&'a self, digest: &'a str) -> BoxFuture<'a, Result<Option<Vec<u8>>>>;

    /// Persist `bytes` under `digest`. Bytes that do not hash to their key
    /// must be refused — a store entry is trusted by its name.
    fn put_content<'a>(&'a self, digest: &'a str, bytes: &'a [u8]) -> BoxFuture<'a, Result<()>>;

    /// The stored release record of `package` at `version` in `registry`.
    fn get_release<'a>(
        &'a self, registry: &'a str, package: &'a str, version: &'a str,
    ) -> BoxFuture<'a, Result<Option<ReleaseRecord>>>;

    /// Persist `record` as the resolution of `package` in `registry`.
    fn put_release<'a>(
        &'a self, registry: &'a str, package: &'a str, record: &'a ReleaseRecord,
    ) -> BoxFuture<'a, Result<()>>;
}

/// The cacheless [`PluginStore`].
///
/// Stores nothing, remembers nothing: every load resolves and fetches fresh,
/// and registry unavailability has no fallback. The default for
/// [`RegistryAcquire`](crate::RegistryAcquire).
#[derive(Clone, Copy, Debug, Default)]
pub struct NoStore;

impl PluginStore for NoStore {
    fn get_content<'a>(&'a self, _digest: &'a str) -> BoxFuture<'a, Result<Option<Vec<u8>>>> {
        async { Ok(None) }.boxed()
    }

    fn put_content<'a>(&'a self, _digest: &'a str, _bytes: &'a [u8]) -> BoxFuture<'a, Result<()>> {
        async { Ok(()) }.boxed()
    }

    fn get_release<'a>(
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
