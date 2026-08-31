//! # Plugin acquisition
//!
//! The [`Acquire`] seam and its built-in acquirers.
//!
//! Acquisition policy — endpoints, cache, path reads — is a value the
//! embedder compiles in at the composition root (the `runtime!` macro's
//! `plugins: { locations: [...] }` list, lowered into the generated
//! `Wiring::acquirer` hook), never runtime-core machinery: the runtime
//! consumes the [`Acquire`] trait and keeps zero storage and network
//! dependencies. This crate is omnia-internal — its surface reaches
//! consumers re-exported from `omnia` under the runtime's own paths.

mod compose;
mod path;
mod registry;
mod store;

use std::fmt;
use std::sync::Arc;

use anyhow::{Context as _, Result, anyhow, ensure};
use futures::future::BoxFuture;

pub use self::compose::{AcquireExt, Or};
pub use self::path::PathAcquire;
pub use self::registry::RegistryAcquire;
pub use self::store::{NoStore, PluginStore, ReleaseRecord, sha256_digest};

/// Where an acquirer finds a package's component bytes — the mirror of the
/// `omnia:plugins/loader` `location` variant.
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

/// One named acquisition root: the location name plus the opened capability
/// handle to its directory.
#[derive(Clone, Debug)]
pub(crate) struct MountEntry {
    /// The location name path loads resolve against.
    pub(crate) name: String,
    /// Host-side capability handle to the location root.
    pub(crate) dir: Arc<cap_std::fs::Dir>,
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
/// runtime's `Wiring::acquirer` hook, which the `runtime!` macro's
/// `plugins: { locations: [...] }` list lowers into.
///
/// An implementation owns every fetch, cache, and endpoint decision; the
/// loader only ever receives bytes back, then verifies, validates, and
/// registers them host-side.
pub trait Acquire: Send + Sync + 'static {
    /// Produce the raw component bytes for `package` from `from`.
    fn acquire<'a>(
        &'a self, package: &'a str, from: &'a Location,
    ) -> BoxFuture<'a, Result<Vec<u8>, AcquireError>>;
}

/// Resolve `path` against `entries` and read the component fresh —
/// [`PathAcquire`]'s read leg.
async fn read_entry(path: &str, entries: &[MountEntry]) -> Result<Vec<u8>, AcquireError> {
    let (dir, subpath) = resolve(path, entries).map_err(AcquireError::Failed)?;
    // File reads are blocking I/O; keep them off the async executor.
    tokio::task::spawn_blocking(move || {
        dir.read(&subpath).with_context(|| format!("reading component `{subpath}`"))
    })
    .await
    .context("component read task panicked")
    .map_err(AcquireError::Failed)?
    .map_err(AcquireError::Failed)
}

/// Resolve `path` to a location's capability handle plus the subpath within
/// it.
///
/// The longest location-name prefix wins; a plain relative path with no
/// matching prefix falls back to a location named `.` when one exists. The
/// subpath must be plain and relative — cap-std then refuses any escape at
/// open time.
fn resolve(path: &str, entries: &[MountEntry]) -> Result<(Arc<cap_std::fs::Dir>, String)> {
    let mut best: Option<(&MountEntry, &str)> = None;
    for entry in entries {
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
        best = entries.iter().find(|entry| entry.name == ".").map(|entry| (entry, path));
    }
    let (entry, subpath) = best
        .ok_or_else(|| anyhow!("path `{path}` is not under any location of this deployment"))?;
    check_subpath(path, subpath)?;
    Ok((Arc::clone(&entry.dir), subpath.to_owned()))
}

// Refuse a subpath that is not a plain relative `/`-separated path.
fn check_subpath(path: &str, subpath: &str) -> Result<()> {
    let plain = !subpath.starts_with('/')
        && !subpath.contains('\\')
        && subpath.split('/').all(|part| !part.is_empty() && part != "." && part != "..");
    ensure!(plain, "component path `{path}` is not a plain relative path within a location");
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use crate::{Acquire as _, AcquireError, Location, PathAcquire};

    fn temp_root(label: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("omnia-acq-{label}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("creating temp location root");
        dir
    }

    async fn acquire(acquirer: &PathAcquire, path: &str) -> Result<Vec<u8>, AcquireError> {
        acquirer.acquire("test:pkg@0.0.1", &Location::Path(path.to_owned())).await
    }

    #[tokio::test]
    async fn longest_location_name_wins() {
        let outer = temp_root("outer");
        let inner = temp_root("outer-inner");
        std::fs::write(inner.join("p.wasm"), b"inner").expect("staging component");
        std::fs::create_dir_all(outer.join("inner")).expect("creating decoy");
        std::fs::write(outer.join("inner").join("p.wasm"), b"outer").expect("staging decoy");
        let acquirer = PathAcquire::new([("adapters", &outer), ("adapters/inner", &inner)])
            .expect("locations open");

        let bytes =
            acquire(&acquirer, "adapters/inner/p.wasm").await.expect("longest prefix reads");
        assert_eq!(bytes, b"inner", "the more specific location serves the path");
    }

    #[tokio::test]
    async fn escape_and_absolute_paths_refused() {
        let root = temp_root("escape");
        let acquirer = PathAcquire::new([(".", &root)]).expect("location opens");

        for path in ["./../secret.wasm", "/etc/passwd", ".\\x.wasm", "./a//b.wasm"] {
            let error = acquire(&acquirer, path).await.expect_err("escape refused");
            assert!(matches!(error, AcquireError::Failed(_)), "path `{path}` must be refused");
        }
    }

    #[tokio::test]
    async fn unlocated_path_and_missing_file_fail() {
        let root = temp_root("missing");
        let acquirer = PathAcquire::new([("adapters", &root)]).expect("location opens");

        let unlocated =
            acquire(&acquirer, "elsewhere/p.wasm").await.expect_err("no location matches");
        assert!(matches!(unlocated, AcquireError::Failed(_)));
        let missing =
            acquire(&acquirer, "adapters/absent.wasm").await.expect_err("file is absent");
        assert!(matches!(missing, AcquireError::Failed(_)));
    }
}
