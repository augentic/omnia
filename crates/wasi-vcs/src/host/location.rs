//! Location resolution in the host.
//!
//! A guest names a repository or working copy as a `location`: a borrowed
//! mount-root descriptor plus a relative subpath. This module turns that
//! into the host path the backend works on, *after* proving the lent root
//! is one the deployment authorized and that the subpath stays beneath it,
//! so an operation can never reach outside the mount.

use std::io;
use std::path::PathBuf;

use anyhow::{Context as _, anyhow, ensure};
use cap_std::fs::{Dir, Metadata, MetadataExt as _};
use omnia_core::MountRegistry;
use wasmtime_wasi::filesystem::Descriptor;

use super::generated::omnia::vcs::types::Location;

// What an operation does at its location.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Intent {
    Read,
    Mutate,
}

// Resolve a lent location into the host path beneath its mount, refusing a
// mutation beneath a read-only mount.
pub fn resolve(
    descriptor: &Descriptor, registry: &MountRegistry, location: &Location, intent: Intent,
) -> anyhow::Result<PathBuf> {
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
        return Ok(entry.host_path.clone());
    }
    check_subpath(&location.subpath)?;
    confine(&entry.dir, &location.subpath)?;
    Ok(entry.host_path.join(&location.subpath))
}

// Refuse a subpath that is not a plain relative `/`-separated path.
fn check_subpath(subpath: &str) -> anyhow::Result<()> {
    let plain = !subpath.starts_with('/')
        && !subpath.contains('\\')
        && subpath.split('/').all(|part| !part.is_empty() && part != "." && part != "..");
    ensure!(plain, "location subpath `{subpath}` is not a plain relative path");
    Ok(())
}

// Open the deepest existing prefix of `subpath` through cap-std, which
// resolves beneath the verified mount root and refuses any escape, so a
// symlink along the way cannot lead the backend out; the remainder does
// not exist yet and is the operation's to create.
fn confine(root: &Dir, subpath: &str) -> anyhow::Result<()> {
    let mut parts: Vec<&str> = subpath.split('/').collect();
    while !parts.is_empty() {
        let prefix = parts.join("/");
        match root.open_dir(&prefix) {
            Ok(_) => return Ok(()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                parts.pop();
            }
            Err(error) => {
                return Err(error).with_context(|| format!("opening location subpath `{prefix}`"));
            }
        }
    }
    Ok(())
}

// subpath vetting and confinement are pure over a directory; the guest
// scenarios cover mount authority and the read-only refusal
#[cfg(test)]
mod tests {
    use std::fs;

    use cap_std::ambient_authority;
    use cap_std::fs::Dir;

    use super::{check_subpath, confine};

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

    #[test]
    fn confine_existing_and_absent() {
        let root = temp_dir("confine");
        fs::create_dir_all(root.join("repos").join("here")).expect("seeding");
        let dir = Dir::open_ambient_dir(&root, ambient_authority()).expect("opening root");

        confine(&dir, "repos/here").unwrap();
        confine(&dir, "repos/here/not/yet").unwrap();
        confine(&dir, "nothing/at/all").unwrap();
        let _ = fs::remove_dir_all(root);
    }

    #[cfg(unix)]
    #[test]
    fn confine_refuses_symlink_out() {
        let root = temp_dir("confine-link");
        let outside = temp_dir("confine-outside");
        std::os::unix::fs::symlink(&outside, root.join("escape")).expect("linking out");
        let dir = Dir::open_ambient_dir(&root, ambient_authority()).expect("opening root");

        confine(&dir, "escape").unwrap_err();
        confine(&dir, "escape/repo").unwrap_err();
        let _ = fs::remove_dir_all(root);
        let _ = fs::remove_dir_all(outside);
    }
}
