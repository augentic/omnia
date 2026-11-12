//! Location resolution in the host.
//!
//! A guest names a repository or working copy as a `location`: a borrowed
//! mount-root descriptor plus a relative subpath. This module turns that
//! into a [`Place`], an open directory handle the backend works through,
//! *after* proving the lent root is one the deployment authorized and
//! opening the subpath beneath it through the mount's own capability, so
//! an operation can never reach outside the mount, however the guest
//! rearranges the tree while the backend runs.

use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use std::{fmt, io};

use anyhow::{Context as _, anyhow, bail, ensure};
use cap_std::fs::{Dir, Metadata, MetadataExt as _};
use omnia_core::MountRegistry;
use wasmtime_wasi::filesystem::Descriptor;

use super::generated::omnia::vcs::types::{Error, Location};

/// The place a guest's `location` resolved to: an open directory beneath an
/// authorized mount.
///
/// The host opens it through the mount's capability handle while the
/// guest's descriptors are borrowed, creating it first for an operation
/// that creates (`init`, `add`, `clone`), and holds it for the backend
/// across every await. The handle is the authority: a guest can rename,
/// remove, or link anything beneath its mount at any moment, so a backend
/// works through [`dir`](Self::dir) and never reopens [`path`](Self::path).
#[derive(Clone)]
pub struct Place {
    dir: Arc<Dir>,
    path: PathBuf,
}

impl Place {
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
}

impl fmt::Debug for Place {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Place").field("path", &self.path).finish_non_exhaustive()
    }
}

// What an operation does at its location.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Intent {
    Read,
    Mutate,
    Create,
}

// Resolve a lent location into the place beneath its mount, refusing a
// mutation beneath a read-only mount.
pub fn resolve(
    descriptor: &Descriptor, registry: &MountRegistry, location: &Location, intent: Intent,
) -> anyhow::Result<Place> {
    let Descriptor::Dir(dir) = descriptor else {
        return Err(anyhow!("location root must be a directory"));
    };

    // cap-std's `Metadata` derives the portable `(dev, ino)` the registry keys on
    let meta = Metadata::from_file(&dir.dir).context("reading lent location root metadata")?;
    let entry = registry
        .match_identity(meta.dev(), meta.ino())
        .context("lent location root is not an authorized mount")?;
    ensure!(
        intent == Intent::Read || entry.writable,
        "mount `{}` is read-only; the operation would write beneath it",
        entry.name
    );

    if location.subpath.is_empty() {
        return Ok(Place {
            dir: Arc::clone(&entry.dir),
            path: entry.host_path.clone(),
        });
    }
    check_subpath(&location.subpath)?;
    let dir = open_beneath(&entry.dir, &location.subpath, intent == Intent::Create)?;
    Ok(Place {
        dir: Arc::new(dir),
        path: entry.host_path.join(&location.subpath),
    })
}

// Refuse a subpath that is not a plain relative `/`-separated path. The
// component walk is what catches a Windows drive prefix such as `C:evil`,
// which is not a child name there.
fn check_subpath(subpath: &str) -> anyhow::Result<()> {
    let plain = !subpath.starts_with('/')
        && !subpath.contains('\\')
        && subpath.split('/').all(|part| !part.is_empty() && part != "." && part != "..")
        && Path::new(subpath).components().all(|part| matches!(part, Component::Normal(_)));
    ensure!(plain, "location subpath `{subpath}` is not a plain relative path");
    Ok(())
}

// Open `subpath` through cap-std, which resolves beneath the verified mount
// root and refuses any escape, creating the missing directories first when
// the operation is one that creates. `create_dir_all` reports a name held
// by anything but a directory it can reach as `AlreadyExists`: a file, a
// dangling symlink, or a symlink that leads out. A subpath nothing holds
// is the typed `not-a-repository` for an operation that does not create,
// carried the way a backend carries its own.
fn open_beneath(root: &Dir, subpath: &str, create: bool) -> anyhow::Result<Dir> {
    if create {
        match root.create_dir_all(subpath) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                bail!("location subpath `{subpath}` is held by something that is not a directory")
            }
            Err(error) => {
                return Err(error)
                    .with_context(|| format!("creating location subpath `{subpath}`"));
            }
        }
    }
    match root.open_dir(subpath) {
        Ok(dir) => Ok(dir),
        Err(error) if !create && error.kind() == io::ErrorKind::NotFound => {
            Err(Error::NotARepository.into())
        }
        Err(error) => Err(error).with_context(|| format!("opening location subpath `{subpath}`")),
    }
}
