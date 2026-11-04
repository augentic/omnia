//! Verified directory handle for `omnia:vcs` operations.

use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use cap_std::fs::Dir;

/// The place a `location` resolved to: an open directory beneath an authorized mount.
#[derive(Clone)]
pub struct Place {
    dir: Arc<Dir>,
    path: PathBuf,
}

impl Place {
    /// Construct a place over a verified directory handle and host path.
    #[must_use]
    pub const fn new(dir: Arc<Dir>, path: PathBuf) -> Self {
        Self { dir, path }
    }

    /// The open directory, confined beneath its mount by cap-std.
    #[must_use]
    pub fn dir(&self) -> &Dir {
        &self.dir
    }

    /// Where the place stood on the host when it resolved.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl fmt::Debug for Place {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Place").field("path", &self.path).finish_non_exhaustive()
    }
}
