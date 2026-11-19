//! Location resolution in the host.
//!
//! A guest names a workspace's directory as a `location`: a borrowed
//! mount-root descriptor plus a relative subpath. This module turns that
//! into a [`Place`], an open directory handle the backend works through,
//! *after* proving the lent root is one the deployment authorized and
//! opening the subpath beneath it through the mount's own capability, so
//! a call can never reach outside the mount, however the guest rearranges
//! the tree while the backend runs.

use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use std::{fmt, io};

use cap_std::fs::{Dir, Metadata, MetadataExt as _};
use omnia_core::MountRegistry;
use wasmtime_wasi::filesystem::Descriptor;

use super::generated::omnia::vcs::types::{Error, Location};

/// The place a guest's `location` resolved to: an open directory beneath an
/// authorized mount.
///
/// The host opens it through the mount's capability handle while the
/// guest's descriptors are borrowed, creating it first for a call that
/// creates (`init`, `add`, `clone`), and holds it for the backend across
/// every await. The handle is the authority: a guest can rename, remove, or
/// link anything beneath its mount at any moment, so a backend works through
/// [`dir`](Self::dir) and never reopens [`path`](Self::path).
#[derive(Clone)]
pub struct Place {
    dir: Arc<Dir>,
    path: PathBuf,
    writable: bool,
}

impl Place {
    // Resolve a lent location into the place beneath its mount.
    pub(crate) fn resolve(
        descriptor: &Descriptor, registry: &MountRegistry, location: &Location, intent: Intent,
    ) -> Result<Self, Error> {
        let Descriptor::Dir(root) = descriptor else {
            return Err(Error::Denied("a location root must be a directory".to_owned()));
        };

        // cap-std's `Metadata` derives the portable `(dev, ino)` the registry keys on
        let meta = Metadata::from_file(&root.dir)
            .map_err(|err| Error::Other(format!("reading a location root's metadata: {err}")))?;
        let Some(mount) = registry.match_identity(meta.dev(), meta.ino()) else {
            return Err(Error::Denied("a location root is not an authorized mount".to_owned()));
        };
        if intent == Intent::Create && !mount.writable {
            return Err(Error::Denied(format!("mount `{}` is read-only", mount.name)));
        }

        let subpath = &location.subpath;
        if subpath.is_empty() {
            return Ok(Self {
                dir: Arc::clone(&mount.dir),
                path: mount.host_path.clone(),
                writable: mount.writable,
            });
        }
        check_subpath(subpath)?;
        let dir = open_beneath(&mount.dir, subpath, intent)?;
        Ok(Self {
            dir: Arc::new(dir),
            path: mount.host_path.join(subpath),
            writable: mount.writable,
        })
    }

    /// The open directory, confined beneath its mount by cap-std.
    ///
    /// Every path the backend opens through it resolves beneath the place
    /// and fails if it would lead out. A process backend hands the handle
    /// to the child (`fchdir` before `exec`, or `/proc/self/fd`) rather than
    /// a path the child would walk again.
    #[must_use]
    pub fn dir(&self) -> &Dir {
        &self.dir
    }

    /// Where the place stood on the host when it resolved.
    ///
    /// For messages and logs only: it is not a capability, and what it
    /// names may have moved by the time a backend reads it.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    pub(crate) const fn writable(&self) -> bool {
        self.writable
    }
}

impl fmt::Debug for Place {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Place").field("path", &self.path).finish_non_exhaustive()
    }
}

// What a call does at its location. Only a call that creates needs a
// writable mount to resolve. A workspace opened beneath a read-only mount
// refuses each change instead.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Intent {
    Open,
    Create,
}

// Refuse a subpath that is not a plain relative `/`-separated path. The
// component walk is what catches a Windows drive prefix such as `C:evil`,
// which is not a child name there.
fn check_subpath(subpath: &str) -> Result<(), Error> {
    let plain = !subpath.starts_with('/')
        && !subpath.contains('\\')
        && subpath.split('/').all(|part| !part.is_empty() && part != "." && part != "..")
        && Path::new(subpath).components().all(|part| matches!(part, Component::Normal(_)));
    if !plain {
        return Err(Error::Invalid(format!(
            "location subpath `{subpath}` is not a plain relative path"
        )));
    }
    Ok(())
}

// Open `subpath` through cap-std, which resolves beneath the verified mount
// root and refuses any escape, making the missing directories first for a
// call that creates. `create_dir_all` reports a name held by anything but a
// directory it can reach as `AlreadyExists`: a file, a dangling symlink, or a
// symlink that leads out.
fn open_beneath(root: &Dir, subpath: &str, intent: Intent) -> Result<Dir, Error> {
    if intent == Intent::Create {
        root.create_dir_all(subpath).map_err(|err| match err.kind() {
            io::ErrorKind::AlreadyExists => Error::Invalid(format!(
                "location subpath `{subpath}` is held by something that is not a directory"
            )),
            io::ErrorKind::PermissionDenied => escapes(subpath),
            _ => Error::Other(format!("creating location subpath `{subpath}`: {err}")),
        })?;
    }
    root.open_dir(subpath).map_err(|err| match err.kind() {
        io::ErrorKind::NotFound | io::ErrorKind::NotADirectory => {
            Error::NotFound(format!("location subpath `{subpath}` holds no directory"))
        }
        io::ErrorKind::PermissionDenied => escapes(subpath),
        _ => Error::Other(format!("opening location subpath `{subpath}`: {err}")),
    })
}

fn escapes(subpath: &str) -> Error {
    Error::Denied(format!("location subpath `{subpath}` leads out of its mount"))
}
