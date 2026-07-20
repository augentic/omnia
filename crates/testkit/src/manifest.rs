//! Writing deployment manifests to a temp file for the duration of a test.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use anyhow::{Context as _, Result};

/// A deployment manifest written to a unique temp file, removed on drop.
///
/// Multi-guest deployments (host-mediated links, routing) are configured by an
/// `omnia.toml`; a test writes one with absolute guest paths so it resolves
/// regardless of the working directory.
#[derive(Debug)]
pub struct TempManifest {
    path: PathBuf,
}

impl TempManifest {
    /// The manifest path, to pass to [`omnia::Manifest::from_config`].
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TempManifest {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

/// Write `contents` to a process- and call-unique `omnia-*.toml` temp file.
///
/// # Errors
///
/// Returns an error if the file cannot be written.
pub fn temp_manifest(contents: &str) -> Result<TempManifest> {
    // A per-call counter keeps concurrent tests in one process from colliding
    // on the pid-based name.
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let seq = SEQ.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!("omnia-{}-{seq}.toml", std::process::id()));
    std::fs::write(&path, contents)
        .with_context(|| format!("writing manifest {}", path.display()))?;
    Ok(TempManifest { path })
}
