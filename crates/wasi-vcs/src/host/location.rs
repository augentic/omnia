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

#[cfg(test)]
mod tests {
    use std::fs;

    use cap_std::ambient_authority;
    use cap_std::fs::Dir;
    use omnia_backend::check_subpath;

    use super::open_beneath;

    fn temp_dir(label: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("omnia-vcs-{label}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("creating temp root");
        dir
    }

    #[test]
    fn plain_subpath() {
        check_subpath("repo").unwrap();
        check_subpath(".emery/vcs/repos/abc").unwrap();
    }

    #[test]
    fn complex_subpath() {
        for subpath in ["/abs", "a\\b", "a//b", ".", "..", "a/../b", "./a", "a/"] {
            check_subpath(subpath).unwrap_err();
        }
    }

    #[cfg(windows)]
    #[test]
    fn drive_prefixed_subpath() {
        for subpath in ["C:evil", "C:/evil"] {
            check_subpath(subpath).unwrap_err();
        }
    }

    #[test]
    fn open_existing_and_absent() {
        let root = temp_dir("open");
        fs::create_dir_all(root.join("repos").join("here")).expect("seeding");
        let dir = Dir::open_ambient_dir(&root, ambient_authority()).expect("opening root");

        open_beneath(&dir, "repos/here", false).unwrap();
        open_beneath(&dir, "repos/here/not/yet", false).unwrap_err();
        open_beneath(&dir, "repos/here/not/yet", true).unwrap();
        assert!(root.join("repos/here/not/yet").is_dir());
        open_beneath(&dir, "repos/here/not/yet", false).unwrap();
        let _ = fs::remove_dir_all(root);
    }

    #[cfg(unix)]
    #[test]
    fn open_refuses_symlink_out() {
        let root = temp_dir("open-link");
        let outside = temp_dir("open-outside");
        std::os::unix::fs::symlink(&outside, root.join("escape")).expect("linking out");
        let dir = Dir::open_ambient_dir(&root, ambient_authority()).expect("opening root");

        for create in [false, true] {
            open_beneath(&dir, "escape", create).unwrap_err();
            open_beneath(&dir, "escape/repo", create).unwrap_err();
        }
        assert!(fs::read_dir(&outside).expect("listing outside").next().is_none());
        let _ = fs::remove_dir_all(root);
        let _ = fs::remove_dir_all(outside);
    }

    #[cfg(unix)]
    #[test]
    fn open_refuses_dangling_symlink() {
        let root = temp_dir("open-dangling");
        std::os::unix::fs::symlink("../../nowhere", root.join("out")).expect("linking out");
        std::os::unix::fs::symlink("nowhere", root.join("within")).expect("linking within");
        let dir = Dir::open_ambient_dir(&root, ambient_authority()).expect("opening root");

        for create in [false, true] {
            open_beneath(&dir, "out", create).unwrap_err();
            open_beneath(&dir, "out/repo", create).unwrap_err();
            open_beneath(&dir, "within", create).unwrap_err();
            open_beneath(&dir, "within/repo", create).unwrap_err();
        }
        assert!(!root.join("nowhere").exists());
        let _ = fs::remove_dir_all(root);
    }
}
