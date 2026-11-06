//! Workspace confinement and bounded I/O for `omnia:model`.

use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context as _, anyhow, ensure};
use cap_std::fs::{Dir, Metadata, MetadataExt as _};
use futures::{FutureExt as _, future};
use omnia_backend::model::DirEntry;
use omnia_backend::{FutureResult, check_subpath};
use omnia_core::{Mount, MountRegistry};
use tokio::task::spawn_blocking;
use wasmtime_wasi::filesystem::Descriptor;

use crate::host::WorkspaceGrant;

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

    pub fn resolve(
        descriptor: &Descriptor, registry: &MountRegistry, grant: &WorkspaceGrant,
    ) -> anyhow::Result<Self> {
        let Descriptor::Dir(dir) = descriptor else {
            return Err(anyhow!("grants.workspace root must be a directory"));
        };

        let meta =
            Metadata::from_file(&dir.dir).context("reading lent workspace directory metadata")?;
        let entry = registry
            .match_identity(meta.dev(), meta.ino())
            .context("lent workspace root is not an authorized mount")?;
        Self::from_mount(entry, &grant.subpath)
    }

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
}

const MAX_READ_BYTES: u64 = 4 * 1024 * 1024;
const MAX_WRITE_BYTES: usize = 4 * 1024 * 1024;
const MAX_LIST_ENTRIES: usize = 4096;
