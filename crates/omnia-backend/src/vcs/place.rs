//! Location confinement for `omnia:vcs`.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::{fmt, io};

use anyhow::{Context as _, anyhow, bail, ensure};
use cap_std::fs::Dir;

use super::Error;
use crate::{Mount, MountRegistry, check_subpath};

/// The place a `location` resolved to: an open directory beneath an authorized mount.
#[derive(Clone)]
pub struct Place {
    dir: Arc<Dir>,
    path: PathBuf,
}

impl Place {
    /// Resolve `subpath` beneath an already-matched [`Mount`] under `intent`.
    ///
    /// # Errors
    ///
    /// Returns an error if `intent` requires writes on a read-only mount,
    /// `subpath` is not a plain relative path, or the directory cannot be
    /// opened (or created when `intent` is [`Intent::Create`]).
    pub fn from_mount(mount: &Mount, subpath: &str, intent: Intent) -> anyhow::Result<Self> {
        ensure!(
            intent == Intent::Read || mount.writable,
            "mount `{}` is read-only; the operation would write beneath it",
            mount.name
        );

        if subpath.is_empty() {
            return Ok(Self {
                dir: Arc::clone(&mount.dir),
                path: mount.host_path.clone(),
            });
        }
        check_subpath(subpath).context("location")?;
        let dir = open_beneath(&mount.dir, subpath, intent == Intent::Create)?;
        Ok(Self {
            dir: Arc::new(dir),
            path: mount.host_path.join(subpath),
        })
    }

    /// Resolve a deployment-local lend `path` against `registry` under `intent`.
    ///
    /// # Errors
    ///
    /// Returns an error if `path` matches no registered mount or [`Self::from_mount`] fails.
    pub fn resolve_path(
        registry: &MountRegistry, path: &str, intent: Intent,
    ) -> anyhow::Result<Self> {
        let (mount, subpath) = registry
            .match_path(path)
            .ok_or_else(|| anyhow!("location `{path}` matches no preopen"))?;
        Self::from_mount(mount, subpath, intent)
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

    use super::open_beneath;
    use crate::check_subpath;

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
