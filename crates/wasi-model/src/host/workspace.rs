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

use anyhow::{Context as _, anyhow, bail, ensure};
use cap_primitives::fs::MetadataExt as _;
use cap_std::fs::Dir;
use futures::{FutureExt as _, future};
use omnia::{FutureResult, MountRegistry};
use tokio::task::spawn_blocking;
use wasmtime::component::ResourceTable;
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

    fn run_blocking<R: Send + 'static>(
        &self, op: &'static str, f: impl FnOnce(&Dir) -> anyhow::Result<R> + Send + 'static,
    ) -> FutureResult<R> {
        let dir = Arc::clone(&self.dir);
        async move {
            spawn_blocking(move || f(&dir))
                .await
                .with_context(|| format!("workspace {op} task failed"))?
        }
        .boxed()
    }

    pub fn read(&self, path: String) -> FutureResult<Vec<u8>> {
        self.run_blocking("read", move |dir| read_blocking(dir, &path))
    }

    pub fn list(&self, path: String) -> FutureResult<Vec<DirEntry>> {
        self.run_blocking("list", move |dir| list_blocking(dir, &path))
    }

    pub fn write(&self, path: String, bytes: Vec<u8>) -> FutureResult<()> {
        if !self.writable {
            return future::err(anyhow!("workspace is read-only; write to `{path}` denied"))
                .boxed();
        }
        self.run_blocking("write", move |dir| write_blocking(dir, &path, &bytes))
    }
}

// Resolve a `grants.workspace` into a [`Workspace`].
pub fn resolve(
    table: &ResourceTable, registry: &MountRegistry, grant: Option<&WorkspaceGrant>,
) -> anyhow::Result<Option<Workspace>> {
    let Some(grant) = grant else {
        return Ok(None);
    };

    let descriptor = table.get(&grant.root).context("resolving the lent workspace descriptor")?;

    let Descriptor::Dir(dir) = descriptor else {
        bail!("grants.workspace root must be a directory descriptor, not a file");
    };

    let meta = dir.dir.dir_metadata().context("reading lent workspace directory metadata")?;
    let entry = registry
        .match_identity(meta.dev(), meta.ino())
        .context("lent workspace root is not an authorized mount (out of scope)")?;

    if grant.subpath.is_empty() {
        return Ok(Some(Workspace {
            dir: Arc::clone(&entry.dir),
            local_path: entry.host_path.clone(),
            writable: entry.writable(),
        }));
    }

    check_subpath(&grant.subpath)?;
    // cap-std's `open_dir` resolves beneath the verified mount root and
    // refuses any escape, so the subpath grant inherits the root's authority.
    let dir = entry
        .dir
        .open_dir(&grant.subpath)
        .with_context(|| format!("opening lent workspace subpath `{}`", grant.subpath))?;

    Ok(Some(Workspace {
        dir: Arc::new(dir),
        local_path: entry.host_path.join(&grant.subpath),
        writable: entry.writable(),
    }))
}

// Refuse a subpath that is not a plain relative `/`-separated path.
fn check_subpath(subpath: &str) -> anyhow::Result<()> {
    let plain = !subpath.starts_with('/')
        && !subpath.contains('\\')
        && subpath.split('/').all(|part| !part.is_empty() && part != "." && part != "..");
    ensure!(plain, "grants.workspace subpath `{subpath}` is not a plain relative path");
    Ok(())
}

fn read_blocking(dir: &Dir, path: &str) -> anyhow::Result<Vec<u8>> {
    let file = dir.open(path).with_context(|| format!("opening `{path}` in workspace"))?;
    // Read one byte past the cap so an over-limit file is detected, not clipped.
    let mut buf = Vec::new();
    file.take(MAX_READ_BYTES + 1)
        .read_to_end(&mut buf)
        .with_context(|| format!("reading `{path}` in workspace"))?;
    ensure!(
        buf.len() as u64 <= MAX_READ_BYTES,
        "file `{path}` exceeds the {MAX_READ_BYTES}-byte workspace read limit"
    );
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
        ensure!(
            entries.len() < MAX_LIST_ENTRIES,
            "directory `{path}` exceeds the {MAX_LIST_ENTRIES}-entry listing limit"
        );

        let is_directory = entry.file_type().is_ok_and(|file_type| file_type.is_dir());
        entries.push(DirEntry {
            name: entry.file_name().to_string_lossy().into_owned(),
            is_directory,
        });
    }
    Ok(entries)
}

fn write_blocking(dir: &Dir, path: &str, bytes: &[u8]) -> anyhow::Result<()> {
    ensure!(
        bytes.len() <= MAX_WRITE_BYTES,
        "write to `{path}` exceeds the {MAX_WRITE_BYTES}-byte workspace write limit"
    );
    dir.write(path, bytes).with_context(|| format!("writing `{path}` in workspace"))
}

// Unit tests by design: subpath vetting is pure validation. cap-std's
// `open_dir` is the runtime escape enforcement; the ABI workspace scenarios
// cover mount authority and write policy.
#[cfg(test)]
mod tests {
    use super::check_subpath;

    #[test]
    fn plain_subpath() {
        check_subpath("docs").unwrap();
        check_subpath("docs/guides").unwrap();
    }

    #[test]
    fn non_plain_subpath() {
        for subpath in ["/abs", "docs\\guides", "docs//guides", ".", "..", "a/../b", "./a", "a/"] {
            check_subpath(subpath).unwrap_err();
        }
    }
}
