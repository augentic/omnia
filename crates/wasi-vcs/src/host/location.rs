//! Location resolution in the host.
//!
//! A guest names a repository or working copy as a `location`: a borrowed
//! mount-root descriptor plus a relative subpath. This module turns that
//! into a [`Place`], an open directory handle the backend works through,
//! *after* proving the lent root is one the deployment authorized and
//! opening the subpath beneath it through the mount's own capability, so
//! an operation can never reach outside the mount, however the guest
//! rearranges the tree while the backend runs.

use std::io;
use std::sync::Arc;

use anyhow::{Context as _, anyhow, bail, ensure};
use cap_std::fs::{Dir, Metadata, MetadataExt as _};
use omnia_backend::check_subpath;
use omnia_backend::vcs::{Error, Place};
use omnia_core::MountRegistry;
use wasmtime_wasi::filesystem::Descriptor;

use super::generated::omnia::vcs::types::Location;

/// What a version-control operation does at its location.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Intent {
    /// Read-only inspection.
    Read,
    /// Mutate an existing repository or working copy.
    Mutate,
    /// Create a new repository or working copy directory.
    Create,
}

pub fn resolve(
    descriptor: &Descriptor, registry: &MountRegistry, location: &Location, intent: Intent,
) -> anyhow::Result<Place> {
    let Descriptor::Dir(dir) = descriptor else {
        return Err(anyhow!("location root must be a directory"));
    };

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
        return Ok(Place::new(Arc::clone(&entry.dir), entry.host_path.clone()));
    }
    check_subpath(&location.subpath).context("location")?;
    let dir = open_beneath(&entry.dir, &location.subpath, intent == Intent::Create)?;
    Ok(Place::new(Arc::new(dir), entry.host_path.join(&location.subpath)))
}

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
