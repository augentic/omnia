//! Digest-keyed persistence for registry-acquired plugin content.

use std::fmt::Write as _;
use std::io::Write as _;
use std::path::PathBuf;

use anyhow::{Context as _, Result, bail};
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

/// A local-directory [`PluginStore`]: `content/<digest>` for the shared
/// content entries, `releases/<registry>/<package>-<version>.json` for the
/// per-registry release records.
///
/// Writes are verify-before-persist — bytes that do not hash to their digest
/// key are refused — and accepted entries land by temp file plus atomic
/// rename, so an entry is either complete or absent, never torn.
#[derive(Clone, Debug)]
pub struct DirStore {
    root: PathBuf,
}

impl DirStore {
    /// Store rooted at `root`; directories are created lazily on first write.
    #[must_use]
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    fn content_path(&self, digest: &str) -> PathBuf {
        self.root.join("content").join(digest)
    }

    fn release_path(&self, registry: &str, package: &str, version: &str) -> PathBuf {
        self.root.join("releases").join(registry).join(format!("{package}-{version}.json"))
    }
}

impl PluginStore for DirStore {
    fn get_content<'a>(&'a self, digest: &'a str) -> BoxFuture<'a, Result<Option<Vec<u8>>>> {
        read_optional(self.content_path(digest)).boxed()
    }

    fn put_content<'a>(&'a self, digest: &'a str, bytes: &'a [u8]) -> BoxFuture<'a, Result<()>> {
        async move {
            // Verify before persist: a mismatched write must never become a
            // digest-keyed entry, even torn.
            let resolved = sha256_digest(bytes);
            if resolved != digest {
                bail!("refusing to persist content keyed {digest}: the bytes hash to {resolved}");
            }
            write_atomic(self.content_path(digest), bytes.to_vec()).await
        }
        .boxed()
    }

    fn get_release<'a>(
        &'a self, registry: &'a str, package: &'a str, version: &'a str,
    ) -> BoxFuture<'a, Result<Option<ReleaseRecord>>> {
        async move {
            let Some(bytes) = read_optional(self.release_path(registry, package, version)).await?
            else {
                return Ok(None);
            };
            let record = serde_json::from_slice(&bytes).context("decoding release record")?;
            Ok(Some(record))
        }
        .boxed()
    }

    fn put_release<'a>(
        &'a self, registry: &'a str, package: &'a str, record: &'a ReleaseRecord,
    ) -> BoxFuture<'a, Result<()>> {
        async move {
            let bytes = serde_json::to_vec(record).context("encoding release record")?;
            write_atomic(self.release_path(registry, package, &record.version), bytes).await
        }
        .boxed()
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

async fn write_atomic(path: PathBuf, bytes: Vec<u8>) -> Result<()> {
    // File writes are blocking I/O; keep them off the async executor.
    tokio::task::spawn_blocking(move || {
        let dir = path.parent().expect("store paths always have a parent");
        std::fs::create_dir_all(dir)
            .with_context(|| format!("creating store directory `{}`", dir.display()))?;
        let mut tmp = tempfile::NamedTempFile::new_in(dir).context("creating store temp file")?;
        tmp.write_all(&bytes).context("writing store temp file")?;
        tmp.persist(&path)
            .with_context(|| format!("persisting store entry `{}`", path.display()))?;
        Ok(())
    })
    .await
    .context("store write task panicked")?
}

async fn read_optional(path: PathBuf) -> Result<Option<Vec<u8>>> {
    match tokio::fs::read(&path).await {
        Ok(bytes) => Ok(Some(bytes)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => {
            Err(anyhow::Error::new(error)
                .context(format!("reading store entry `{}`", path.display())))
        }
    }
}
