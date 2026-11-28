//! The workspace a guest lends to a completion, as a backend reaches it.

use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use cap_std::fs::Dir;

/// The workspace a guest lent to a completion: an open directory beneath an
/// authorized mount.
///
/// The host opens it through the mount's capability handle as the completion
/// begins, and holds it for the backend until the completion ends. The handle
/// is the authority: a guest can rename, remove, or link anything beneath its
/// mount at any moment, so a backend works through [`dir`](Self::dir) and
/// never reopens [`path`](Self::path).
#[derive(Clone)]
pub struct Lend {
    dir: Arc<Dir>,
    path: PathBuf,
    writable: bool,
}

impl Lend {
    /// A lend of `dir`, shown as `path`, that the backend may write where
    /// `writable` holds.
    ///
    /// The host builds one for each completion that lends a workspace. A
    /// [`ToolHost`](crate::ToolHost) implemented outside the host builds its
    /// own.
    #[must_use]
    pub fn new(dir: impl Into<Arc<Dir>>, path: PathBuf, writable: bool) -> Self {
        Self {
            dir: dir.into(),
            path,
            writable,
        }
    }

    /// The open directory, confined beneath its mount by cap-std.
    ///
    /// Every path the backend opens through it resolves beneath the lend and
    /// fails if it would lead out. A process backend hands the handle to the
    /// child, and binds the child's kernel boundary to it, rather than to a
    /// path the child would walk again.
    #[must_use]
    pub fn dir(&self) -> &Dir {
        &self.dir
    }

    /// Where the lend stood on the host when it resolved.
    ///
    /// For messages and logs only: it is not a capability, and what it names
    /// may have moved by the time a backend reads it.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Whether the lend lies beneath a writable mount.
    ///
    /// A backend's processes write the lend only where it does.
    #[must_use]
    pub const fn writable(&self) -> bool {
        self.writable
    }

    pub(crate) const fn shared(&self) -> &Arc<Dir> {
        &self.dir
    }
}

impl fmt::Debug for Lend {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Lend")
            .field("path", &self.path)
            .field("writable", &self.writable)
            .finish_non_exhaustive()
    }
}
