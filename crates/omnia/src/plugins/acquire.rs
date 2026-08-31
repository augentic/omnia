//! The `Acquire` seam and the preopen-relative `MountAcquire`.
//!
//! Acquisition policy — endpoints, cache, path reads — is a value the
//! embedder compiles in at the composition root, never core machinery: core
//! consumes the [`Acquire`] trait and keeps zero storage and network
//! dependencies. [`MountAcquire`] is the battery core ships (fresh reads over
//! the existing mount registry); registry acquirers are embedder territory.

use std::fmt;
use std::sync::Arc;

use anyhow::{Context as _, Result, anyhow, ensure};
use futures::FutureExt as _;
use futures::future::BoxFuture;

use crate::mount::MountRegistry;

/// Where an acquirer finds a package's component bytes — the core mirror of
/// the `omnia:plugins/loader` `location` variant.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Location {
    /// A package registry; `None` selects the acquirer's default.
    Registry(Option<String>),
    /// A preopen-relative component path, read fresh on every load.
    Path(String),
}

impl fmt::Display for Location {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Registry(None) => f.write_str("the default registry"),
            Self::Registry(Some(registry)) => write!(f, "registry `{registry}`"),
            Self::Path(path) => write!(f, "path `{path}`"),
        }
    }
}

/// Deployment facts the runtime lends an acquirer per load.
///
/// Mounts open after the composition root constructs the acquirer, so they
/// arrive per call rather than at construction — `acquire: MountAcquire`
/// stays a plain value in the `runtime!` declaration.
pub struct AcquireContext {
    /// The deployment's mount registry, for preopen-relative reads.
    pub mounts: Arc<MountRegistry>,
}

/// Why an acquirer produced no bytes.
#[derive(Debug)]
pub enum AcquireError {
    /// The acquirer does not serve this location kind.
    Unsupported(String),
    /// The location is served, but the package could not be produced.
    Failed(anyhow::Error),
}

/// Acquisition policy compiled in at the composition root — built by the
/// [`Wiring::acquirer`](crate::Wiring::acquirer) hook, which the `runtime!`
/// macro's `plugins: { acquire: ... }` key lowers into.
///
/// An implementation owns every fetch, cache, and endpoint decision; the
/// loader only ever receives bytes back, then verifies, validates, and
/// registers them host-side.
pub trait Acquire: Send + Sync + 'static {
    /// Produce the raw component bytes for `package` from `from`.
    fn acquire<'a>(
        &'a self, package: &'a str, from: &'a Location, context: &'a AcquireContext,
    ) -> BoxFuture<'a, Result<Vec<u8>, AcquireError>>;
}

/// Preopen-relative path acquisition over the deployment's mount registry.
///
/// Paths resolve against the registered mounts (longest mount-name prefix
/// wins; a bare relative path falls back to a `.` mount, matching wasi-libc's
/// preopen resolution) and are read fresh on every load — never cached.
/// Registry locations are refused: composing a registry acquirer is
/// deployment policy outside core.
#[derive(Clone, Copy, Debug, Default)]
pub struct MountAcquire;

impl Acquire for MountAcquire {
    fn acquire<'a>(
        &'a self, package: &'a str, from: &'a Location, context: &'a AcquireContext,
    ) -> BoxFuture<'a, Result<Vec<u8>, AcquireError>> {
        async move {
            let Location::Path(path) = from else {
                return Err(AcquireError::Unsupported(format!(
                    "MountAcquire reads preopen-relative paths only; acquiring `{package}` \
                     from {from} requires a registry acquirer"
                )));
            };
            let (dir, subpath) = resolve(path, &context.mounts).map_err(AcquireError::Failed)?;
            // File reads are blocking I/O; keep them off the async executor.
            tokio::task::spawn_blocking(move || {
                dir.read(&subpath).with_context(|| format!("reading component `{subpath}`"))
            })
            .await
            .context("component read task panicked")
            .map_err(AcquireError::Failed)?
            .map_err(AcquireError::Failed)
        }
        .boxed()
    }
}

/// Resolve `path` to a mount's capability handle plus the subpath within it.
///
/// The longest mount-name prefix wins; a plain relative path with no matching
/// prefix falls back to a mount named `.` when one exists. The subpath must
/// be plain and relative — cap-std then refuses any escape at open time.
fn resolve(path: &str, mounts: &MountRegistry) -> Result<(Arc<cap_std::fs::Dir>, String)> {
    let mut best: Option<(&crate::mount::Mount, &str)> = None;
    for entry in mounts.entries() {
        let subpath = if path == entry.name {
            ""
        } else if let Some(rest) = path.strip_prefix(&entry.name).and_then(|r| r.strip_prefix('/'))
        {
            rest
        } else {
            continue;
        };
        if best.is_none_or(|(current, _)| entry.name.len() > current.name.len()) {
            best = Some((entry, subpath));
        }
    }
    if best.is_none() {
        // wasi-libc resolves bare relative paths against a `.` preopen; do
        // the same so host- and guest-side views of a path agree.
        best = mounts.entries().iter().find(|entry| entry.name == ".").map(|entry| (entry, path));
    }
    let (entry, subpath) =
        best.ok_or_else(|| anyhow!("path `{path}` is not under any mount of this deployment"))?;
    check_subpath(path, subpath)?;
    Ok((Arc::clone(&entry.dir), subpath.to_owned()))
}

// Refuse a subpath that is not a plain relative `/`-separated path.
fn check_subpath(path: &str, subpath: &str) -> Result<()> {
    let plain = !subpath.starts_with('/')
        && !subpath.contains('\\')
        && subpath.split('/').all(|part| !part.is_empty() && part != "." && part != "..");
    ensure!(plain, "component path `{path}` is not a plain relative path within a mount");
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::Arc;

    use crate::mount::{MountRegistry, ResolvedPreopen};
    use crate::plugins::{Acquire as _, AcquireContext, AcquireError, Location, MountAcquire};

    fn temp_root(label: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("omnia-acq-{label}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("creating temp mount root");
        dir
    }

    fn context(mounts: &[(&str, &PathBuf)]) -> AcquireContext {
        let preopens = mounts
            .iter()
            .map(|(name, path)| ResolvedPreopen::new((*name).to_owned(), (*path).clone(), false))
            .collect();
        AcquireContext {
            mounts: Arc::new(MountRegistry::open(preopens).expect("opening mounts")),
        }
    }

    async fn acquire(context: &AcquireContext, path: &str) -> Result<Vec<u8>, AcquireError> {
        MountAcquire.acquire("test:pkg@0.0.1", &Location::Path(path.to_owned()), context).await
    }

    #[tokio::test]
    async fn dot_mount_serves_prefixed_and_bare_paths() {
        let root = temp_root("dot");
        std::fs::write(root.join("plugin.wasm"), b"bytes").expect("staging component");
        let context = context(&[(".", &root)]);

        let prefixed = acquire(&context, "./plugin.wasm").await.expect("prefixed path reads");
        assert_eq!(prefixed, b"bytes");
        let bare = acquire(&context, "plugin.wasm").await.expect("bare path reads");
        assert_eq!(bare, b"bytes");
    }

    #[tokio::test]
    async fn longest_mount_name_wins() {
        let outer = temp_root("outer");
        let inner = temp_root("outer-inner");
        std::fs::write(inner.join("p.wasm"), b"inner").expect("staging component");
        std::fs::create_dir_all(outer.join("inner")).expect("creating decoy");
        std::fs::write(outer.join("inner").join("p.wasm"), b"outer").expect("staging decoy");
        let context = context(&[("adapters", &outer), ("adapters/inner", &inner)]);

        let bytes = acquire(&context, "adapters/inner/p.wasm").await.expect("longest prefix reads");
        assert_eq!(bytes, b"inner", "the more specific mount serves the path");
    }

    #[tokio::test]
    async fn escape_and_absolute_paths_refused() {
        let root = temp_root("escape");
        let context = context(&[(".", &root)]);

        for path in ["./../secret.wasm", "/etc/passwd", ".\\x.wasm", "./a//b.wasm"] {
            let error = acquire(&context, path).await.expect_err("escape refused");
            assert!(matches!(error, AcquireError::Failed(_)), "path `{path}` must be refused");
        }
    }

    #[tokio::test]
    async fn unmounted_path_and_missing_file_fail() {
        let root = temp_root("missing");
        let context = context(&[("adapters", &root)]);

        let unmounted = acquire(&context, "elsewhere/p.wasm").await.expect_err("no mount matches");
        assert!(matches!(unmounted, AcquireError::Failed(_)));
        let missing = acquire(&context, "adapters/absent.wasm").await.expect_err("file is absent");
        assert!(matches!(missing, AcquireError::Failed(_)));
    }

    #[tokio::test]
    async fn registry_location_unsupported() {
        let root = temp_root("registry");
        let context = context(&[(".", &root)]);

        let error = MountAcquire
            .acquire("test:pkg@0.0.1", &Location::Registry(None), &context)
            .await
            .expect_err("registry locations are not served");
        assert!(matches!(error, AcquireError::Unsupported(_)));
    }
}
