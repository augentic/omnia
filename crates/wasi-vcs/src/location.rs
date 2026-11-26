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
use crate::Error as BackendError;

/// The place a guest's `location` resolved to: an open directory beneath an
/// authorized mount.
///
/// The host opens it through the mount's capability handle while the
/// guest's descriptors are borrowed, creating it first for a call that
/// creates (`init`, `add`, `clone`), and holds it for the backend across
/// every await. The handle is the authority: a guest can rename, remove, or
/// link anything beneath its mount at any moment, so a backend works through
/// [`dir`](Self::dir) and never reopens [`path`](Self::path).
///
/// A place carries the mounts its guest holds. A backend holds every other
/// directory a call reaches to them through [`locate`](Self::locate).
#[derive(Clone)]
pub struct Place {
    dir: Arc<Dir>,
    path: PathBuf,
    writable: bool,
    mounts: Arc<MountRegistry>,
}

impl Place {
    // Resolve a lent location into the place beneath its mount.
    pub(crate) fn resolve(
        descriptor: &Descriptor, registry: &Arc<MountRegistry>, location: &Location, intent: Intent,
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
                mounts: Arc::clone(registry),
            });
        }
        check_subpath(subpath)?;
        let dir = open_beneath(&mount.dir, subpath, intent)?;
        Ok(Self {
            dir: Arc::new(dir),
            path: mount.host_path.join(subpath),
            writable: mount.writable,
            mounts: Arc::clone(registry),
        })
    }

    /// The directory at `path`, held to the [`mounts`](Self::mounts) the
    /// guest holds.
    ///
    /// A backend locates each directory a call reaches beyond the place
    /// itself, a local address included, before anything reads it.
    ///
    /// - A relative `path` is joined to this place's [`path`](Self::path).
    /// - `path` is placed by directory identity, so a symlink that leads out
    ///   of the mounts places it outside them.
    /// - With `write`, `path` must lie beneath a writable mount.
    /// - The directory is opened through its mount's handle, so a symlink
    ///   swapped in meanwhile cannot lead the open out.
    ///
    /// # Errors
    ///
    /// - [`BackendError::Denied`], naming `path`, when it lies beneath no
    ///   mount the guest holds, whether or not it exists, or beneath no
    ///   writable one where `write` needs it, or leads out of its mount as it
    ///   opens;
    /// - [`BackendError::NotFound`], naming `path`, when nothing at `path`
    ///   beneath a mount the guest holds is a directory;
    /// - [`BackendError::Other`] when `path` cannot be placed.
    pub fn locate(&self, path: &Path, write: bool) -> Result<Self, BackendError> {
        let shown = path.display();
        let joined = self.path.join(path);
        let unplaced =
            |err: anyhow::Error| BackendError::Other(format!("placing {shown}: {err:#}"));

        // beneath a held mount, and a writable one for a write
        let found = self.mounts.beneath(&joined, |mount| mount.writable || !write);
        let Some((mount, rest)) = found.map_err(unplaced)? else {
            let held = write && self.mounts.beneath(&joined, |_| true).map_err(unplaced)?.is_some();
            let why = if held { "beneath no writable mount" } else { "outside the guest's mounts" };
            return Err(BackendError::Denied(format!("{shown} lies {why}")));
        };

        // the directory itself, opened through the mount's handle
        let (dir, path) = if rest.as_os_str().is_empty() {
            (Arc::clone(&mount.dir), mount.host_path.clone())
        } else {
            let dir = mount.dir.open_dir(&rest).map_err(|err| match err.kind() {
                io::ErrorKind::NotFound | io::ErrorKind::NotADirectory => {
                    BackendError::NotFound(format!("{shown} holds no directory"))
                }
                io::ErrorKind::PermissionDenied => {
                    BackendError::Denied(format!("{shown} leads out of its mount"))
                }
                _ => BackendError::Other(format!("opening {shown}: {err}")),
            })?;
            (Arc::new(dir), mount.host_path.join(&rest))
        };
        Ok(Self {
            dir,
            path,
            writable: mount.writable,
            mounts: Arc::clone(&self.mounts),
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

    /// Whether the place lies beneath a writable mount.
    #[must_use]
    pub const fn writable(&self) -> bool {
        self.writable
    }

    /// The mounts the store's guest holds, as its grants narrow them.
    #[must_use]
    pub fn mounts(&self) -> &MountRegistry {
        &self.mounts
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
