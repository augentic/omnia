//! Workspace confinement and bounded I/O for `omnia:model`.

use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context as _, anyhow, ensure};
use cap_std::fs::Dir;
use futures::{FutureExt as _, future};
use tokio::task::spawn_blocking;

use super::{DirEntry, Error};
use crate::{FutureResult, Mount, MountRegistry, check_subpath};

const MAX_READ_BYTES: u64 = 4 * 1024 * 1024;
const MAX_WRITE_BYTES: usize = 4 * 1024 * 1024;
const MAX_LIST_ENTRIES: usize = 4096;

/// A resolved workspace mount confined beneath an authorized [`Mount`].
#[derive(Clone)]
pub struct Workspace {
    dir: Arc<Dir>,
    local_path: PathBuf,
    writable: bool,
}

impl std::fmt::Debug for Workspace {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Workspace")
            .field("local_path", &self.local_path)
            .field("writable", &self.writable)
            .finish_non_exhaustive()
    }
}

impl Workspace {
    /// Resolve `subpath` beneath an already-matched [`Mount`].
    ///
    /// # Errors
    ///
    /// Returns an error if `subpath` is not a plain relative path or cannot be
    /// opened as a directory beneath `mount`.
    pub fn from_mount(mount: &Mount, subpath: &str) -> anyhow::Result<Self> {
        if subpath.is_empty() {
            return Ok(Self {
                dir: Arc::clone(&mount.dir),
                local_path: mount.host_path.clone(),
                writable: mount.writable,
            });
        }

        check_subpath(subpath).context("grants.workspace")?;
        let dir = mount
            .dir
            .open_dir(subpath)
            .with_context(|| format!("opening lent workspace subpath `{subpath}`"))?;

        Ok(Self {
            dir: Arc::new(dir),
            local_path: mount.host_path.join(subpath),
            writable: mount.writable,
        })
    }

    /// Resolve a deployment-local lend `path` against `registry`.
    ///
    /// Returns the confined [`Workspace`] and the matched mount-relative `subpath`.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidRequest`] if `path` matches no registered mount
    /// or [`Error::Backend`] if the subpath cannot be opened beneath the
    /// matched mount.
    pub fn resolve_path<'a>(
        registry: &MountRegistry, path: &'a str,
    ) -> Result<(Self, &'a str), Error> {
        let (mount, subpath) = registry.match_path(path).ok_or_else(|| {
            Error::InvalidRequest(format!("workspace lend `{path}` matches no preopen"))
        })?;
        let workspace =
            Self::from_mount(mount, subpath).map_err(|e| Error::Backend(format!("{e:#}")))?;
        Ok((workspace, subpath))
    }

    /// The absolute host path of the workspace when it resolved.
    #[must_use]
    pub fn local_path(&self) -> &Path {
        &self.local_path
    }

    /// Bounded read of `path` inside the workspace.
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

    /// Bounded directory listing of `path` inside the workspace.
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

    /// Bounded write of `bytes` to `path` inside a writable workspace.
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

#[cfg(test)]
mod tests {
    use std::fs;

    use cap_std::ambient_authority;
    use cap_std::fs::Dir;

    use super::{MAX_LIST_ENTRIES, MAX_READ_BYTES, MAX_WRITE_BYTES, Workspace};
    use crate::check_subpath;

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
