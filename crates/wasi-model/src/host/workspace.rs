//! Workspace resolution in the host.
//!
//! A guest lends a `wasi:filesystem` workspace through
//! `grants.workspace: option<borrow<descriptor>>`. This module turns that
//! borrowed descriptor into an owned, `Send + Sync` [`Workspace`] the backend
//! can use across `.await` points, *after* proving the lent directory is one the
//! deployment authorized.

use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context as _, anyhow, bail};
use cap_primitives::fs::MetadataExt as _;
use cap_std::fs::Dir;
use futures::FutureExt as _;
use omnia::{FutureResult, MountRegistry};
use tokio::task::spawn_blocking;
use wasmtime::component::{Resource, ResourceTable};
use wasmtime_wasi::filesystem::Descriptor;

use super::types::DirEntry;

const MAX_READ_BYTES: u64 = 4 * 1024 * 1024;
const MAX_WRITE_BYTES: usize = 4 * 1024 * 1024;
const MAX_LIST_ENTRIES: usize = 4096;

// An handle to a resolved workspace mount. Built by [`resolve`].
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

    // Off-thread a bounded, blocking cap-std op against the mount, tagging a
    // task-join failure with `op`.
    fn run_blocking<R: Send + 'static>(
        &self, f: impl FnOnce(&Dir) -> anyhow::Result<R> + Send + 'static,
    ) -> FutureResult<R> {
        let dir = Arc::clone(&self.dir);
        async move { spawn_blocking(move || f(&dir)).await.context("workspace read task failed")? }
            .boxed()
    }

    pub fn read(&self, path: String) -> FutureResult<Vec<u8>> {
        self.run_blocking(move |dir| read_blocking(dir, &path))
    }

    pub fn list(&self, path: String) -> FutureResult<Vec<DirEntry>> {
        self.run_blocking(move |dir| list_blocking(dir, &path))
    }

    pub fn write(&self, path: String, bytes: Vec<u8>) -> FutureResult<()> {
        if !self.writable {
            return ready_err(anyhow!("workspace is read-only; write to `{path}` denied"));
        }
        self.run_blocking(move |dir| write_blocking(dir, &path, &bytes))
    }
}

// A ready future already resolved to `err`.
fn ready_err<R: Send + 'static>(err: anyhow::Error) -> FutureResult<R> {
    async move { Err(err) }.boxed()
}

// Resolve a `grants.workspace` into a [`Workspace`].
pub fn resolve(
    table: &ResourceTable, registry: &MountRegistry, borrow: Option<&Resource<Descriptor>>,
) -> anyhow::Result<Option<Workspace>> {
    let Some(resource) = borrow else {
        return Ok(None);
    };

    let descriptor = table.get(resource).context("resolving the lent workspace descriptor")?;

    let Descriptor::Dir(dir) = descriptor else {
        bail!("grants.workspace must be a directory descriptor, not a file");
    };

    let meta = dir.dir.dir_metadata().context("reading lent workspace directory metadata")?;
    let entry = registry
        .match_identity(meta.dev(), meta.ino())
        .context("lent workspace is not an authorized mount (out of scope)")?;

    Ok(Some(Workspace {
        dir: Arc::clone(&entry.dir),
        local_path: entry.host_path.clone(),
        writable: entry.writable(),
    }))
}

fn read_blocking(dir: &Dir, path: &str) -> anyhow::Result<Vec<u8>> {
    let file = dir.open(path).with_context(|| format!("opening `{path}` in workspace"))?;
    // Read one byte past the cap so an over-limit file is detected, not clipped.
    let mut buf = Vec::new();
    file.take(MAX_READ_BYTES + 1)
        .read_to_end(&mut buf)
        .with_context(|| format!("reading `{path}` in workspace"))?;
    if buf.len() as u64 > MAX_READ_BYTES {
        bail!("file `{path}` exceeds the {MAX_READ_BYTES}-byte workspace read limit");
    }
    Ok(buf)
}

fn list_blocking(dir: &Dir, path: &str) -> anyhow::Result<Vec<DirEntry>> {
    let read_dir = if path.is_empty() || path == "." {
        dir.entries().context("listing workspace root")?
    } else {
        dir.read_dir(path).with_context(|| format!("listing `{path}` in workspace"))?
    };

    let mut entries = Vec::new();
    for entry in read_dir {
        let entry = entry.context("reading workspace directory entry")?;
        if entries.len() >= MAX_LIST_ENTRIES {
            bail!("directory `{path}` exceeds the {MAX_LIST_ENTRIES}-entry listing limit");
        }

        let is_directory = entry.file_type().is_ok_and(|file_type| file_type.is_dir());
        entries.push(DirEntry {
            name: entry.file_name().to_string_lossy().into_owned(),
            is_directory,
        });
    }
    Ok(entries)
}

fn write_blocking(dir: &Dir, path: &str, bytes: &[u8]) -> anyhow::Result<()> {
    if bytes.len() > MAX_WRITE_BYTES {
        bail!("write to `{path}` exceeds the {MAX_WRITE_BYTES}-byte workspace write limit");
    }
    dir.write(path, bytes).with_context(|| format!("writing `{path}` in workspace"))
}
