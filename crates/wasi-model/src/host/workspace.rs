//! Workspace resolution in the host.
//!
//! A guest lends a `wasi:filesystem` workspace through
//! `grants.workspace: option<workspace-grant>` — a borrowed mount-root
//! descriptor plus a relative subpath. This module turns that grant into an
//! owned, `Send + Sync` [`Workspace`] the backend can use across `.await`
//! points, *after* proving the lent root is one the deployment authorized
//! and reopening the subpath beneath it so the lend can never escape the
//! mount.

use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context as _, anyhow, ensure};
use cap_std::fs::{Dir, Metadata, MetadataExt as _};
use futures::{FutureExt as _, future};
use omnia::{FutureResult, MountRegistry};
use tokio::task::spawn_blocking;
use wasmtime_wasi::filesystem::Descriptor;

use super::generated::omnia::model::completion::WorkspaceGrant;
use super::tool_host::DirEntry;

const MAX_READ_BYTES: u64 = 4 * 1024 * 1024;
const MAX_WRITE_BYTES: usize = 4 * 1024 * 1024;
const MAX_LIST_ENTRIES: usize = 4096;

// A resolved workspace mount.
pub struct Workspace {
    dir: Arc<Dir>,
    local_path: PathBuf,
    writable: bool,
}

impl Workspace {
    #[must_use]
    pub fn local_path(&self) -> &Path {
        &self.local_path
    }

    pub fn read(&self, path: String) -> FutureResult<Vec<u8>> {
        self.with_dir(move |dir| {
            let file = dir.open(&path).context("opening path in workspace")?;
            let mut buf = Vec::new();
            file.take(MAX_READ_BYTES + 1)
                .read_to_end(&mut buf)
                .context("reading path in workspace")?;

            ensure!(
                buf.len() as u64 <= MAX_READ_BYTES,
                "file `{path}` exceeds workspace read limit"
            );
            Ok(buf)
        })
    }

    pub fn list(&self, path: String) -> FutureResult<Vec<DirEntry>> {
        self.with_dir(move |dir| {
            let read_dir = if path.is_empty() || path == "." {
                dir.entries().context("listing workspace root")?
            } else {
                dir.read_dir(&path).context("listing path in workspace")?
            };

            let mut entries = Vec::new();
            for entry in read_dir {
                let entry = entry.context("reading workspace directory entry")?;
                ensure!(entries.len() < MAX_LIST_ENTRIES, "directory exceeds listing limit");

                let is_directory = entry.file_type().is_ok_and(|file_type| file_type.is_dir());
                entries.push(DirEntry {
                    name: entry.file_name().to_string_lossy().into_owned(),
                    is_directory,
                });
            }
            Ok(entries)
        })
    }

    pub fn write(&self, path: String, bytes: Vec<u8>) -> FutureResult<()> {
        if !self.writable {
            return future::err(anyhow!("workspace is read-only")).boxed();
        }
        self.with_dir(move |dir| {
            ensure!(bytes.len() <= MAX_WRITE_BYTES, "write limit exceeded");
            dir.write(&path, &bytes).with_context(|| format!("writing `{path}` in workspace"))
        })
    }

    fn with_dir<R: Send + 'static>(
        &self, f: impl FnOnce(&Dir) -> anyhow::Result<R> + Send + 'static,
    ) -> FutureResult<R> {
        let dir = Arc::clone(&self.dir);
        async move { spawn_blocking(move || f(&dir)).await? }.boxed()
    }

    #[cfg(test)]
    fn test_handle(dir: Dir, writable: bool) -> Self {
        Self {
            dir: Arc::new(dir),
            local_path: PathBuf::from("/test"),
            writable,
        }
    }
}

// Resolve a lent workspace directory and grant into a [`Workspace`].
pub fn resolve(
    descriptor: &Descriptor, registry: &MountRegistry, grant: &WorkspaceGrant,
) -> anyhow::Result<Workspace> {
    let Descriptor::Dir(dir) = descriptor else {
        return Err(anyhow!("grants.workspace root must be a directory"));
    };

    // The lent handle is a plain `std::fs::File`; cap-std's `Metadata` (the
    // same type the registry's `dir_metadata()` produces) derives the
    // portable `(dev, ino)` identity the registry keys on.
    let meta =
        Metadata::from_file(&dir.dir).context("reading lent workspace directory metadata")?;
    let entry = registry
        .match_identity(meta.dev(), meta.ino())
        .context("lent workspace root is not an authorized mount")?;

    if grant.subpath.is_empty() {
        return Ok(Workspace {
            dir: Arc::clone(&entry.dir),
            local_path: entry.host_path.clone(),
            writable: entry.writable,
        });
    }

    check_subpath(&grant.subpath)?;
    // cap-std's `open_dir` resolves beneath the verified mount root and
    // refuses any escape, so the subpath grant inherits the root's authority.
    let dir = entry
        .dir
        .open_dir(&grant.subpath)
        .with_context(|| format!("opening lent workspace subpath `{}`", grant.subpath))?;

    Ok(Workspace {
        dir: Arc::new(dir),
        local_path: entry.host_path.join(&grant.subpath),
        writable: entry.writable,
    })
}

// Refuse a subpath that is not a plain relative `/`-separated path.
fn check_subpath(subpath: &str) -> anyhow::Result<()> {
    let plain = !subpath.starts_with('/')
        && !subpath.contains('\\')
        && subpath.split('/').all(|part| !part.is_empty() && part != "." && part != "..");
    ensure!(plain, "grants.workspace subpath `{subpath}` is not a plain relative path");
    Ok(())
}

// Unit tests by design: subpath vetting is pure validation; size/listing
// caps and subdirectory listing are host I/O bounds no guest uniquely
// observes. cap-std's `open_dir` is the runtime escape enforcement; the ABI
// workspace scenarios cover mount authority and write policy.
#[cfg(test)]
mod tests {
    use std::fs;

    use cap_std::ambient_authority;
    use cap_std::fs::Dir;

    use super::{MAX_LIST_ENTRIES, MAX_READ_BYTES, MAX_WRITE_BYTES, Workspace, check_subpath};

    fn temp_dir(label: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("omnia-ws-{label}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("creating temp workspace");
        dir
    }

    fn handle(label: &str, writable: bool) -> (std::path::PathBuf, Workspace) {
        let path = temp_dir(label);
        let dir = Dir::open_ambient_dir(&path, ambient_authority()).expect("opening temp dir");
        (path, Workspace::test_handle(dir, writable))
    }

    #[test]
    fn plain_subpath() {
        check_subpath("docs").unwrap();
        check_subpath("docs/guides").unwrap();
    }

    #[test]
    fn complex_subpath() {
        for subpath in ["/abs", "docs\\guides", "docs//guides", ".", "..", "a/../b", "./a", "a/"] {
            check_subpath(subpath).unwrap_err();
        }
    }

    #[tokio::test]
    async fn list_subdirectory() {
        let (root, workspace) = handle("list-subdir", false);
        fs::create_dir(root.join("docs")).expect("creating docs");
        fs::write(root.join("docs").join("a.txt"), "a").expect("seeding docs");
        fs::write(root.join("root.txt"), "r").expect("seeding root");

        let mut entries = workspace.list("docs".to_owned()).await.expect("listing docs");
        entries.sort_by(|left, right| left.name.cmp(&right.name));
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name, "a.txt");
        assert!(!entries[0].is_directory);

        let mut root_entries = workspace.list(String::new()).await.expect("listing root");
        root_entries.sort_by(|left, right| left.name.cmp(&right.name));
        let docs = root_entries.iter().find(|entry| entry.name == "docs").expect("docs dir");
        assert!(docs.is_directory);
        let _ = fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn read_over_limit() {
        let (root, workspace) = handle("read-limit", false);
        let oversized =
            vec![0_u8; usize::try_from(MAX_READ_BYTES).expect("read cap fits usize") + 1];
        fs::write(root.join("big.bin"), &oversized).expect("writing oversized file");
        let error = workspace.read("big.bin".to_owned()).await.expect_err("read limit");
        assert!(error.to_string().contains("exceeds workspace read limit"), "{error}");
        let _ = fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn write_over_limit() {
        let (root, workspace) = handle("write-limit", true);
        let oversized = vec![0_u8; MAX_WRITE_BYTES + 1];
        let error =
            workspace.write("big.bin".to_owned(), oversized).await.expect_err("write limit");
        assert!(error.to_string().contains("write limit exceeded"), "{error}");
        let _ = fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn list_over_limit() {
        let (root, workspace) = handle("list-limit", true);
        for i in 0..=MAX_LIST_ENTRIES {
            fs::write(root.join(format!("{i}.txt")), []).expect("seeding list entries");
        }
        let error = workspace.list(String::new()).await.expect_err("list limit");
        assert!(error.to_string().contains("directory exceeds listing limit"), "{error}");
        let _ = fs::remove_dir_all(root);
    }
}
