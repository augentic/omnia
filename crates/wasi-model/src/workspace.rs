//! Workspace resolution in the host.
//!
//! A guest lends a `wasi:filesystem` workspace through the `location` of
//! `grants.workspace` — a borrowed mount-root descriptor plus a relative
//! subpath. This module turns that location into an owned,
//! `Send + Sync` [`Workspace`] the backend can use across `.await` points,
//! *after* proving the lent root is one the deployment authorized and
//! reopening the subpath beneath it so the lend can never escape the mount.

use std::io::{self, Read as _};
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context as _, anyhow, ensure};
use cap_std::fs::{Dir, Metadata, MetadataExt as _};
use futures::{FutureExt as _, future};
use omnia_core::{FutureResult, MountRegistry};
use tokio::task::spawn_blocking;
use wasmtime_wasi::filesystem::Descriptor;

use super::tool_host::DirEntry;
use super::{Error, Location};

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
}

// Resolve a lent workspace directory and location into a [`Workspace`].
pub fn resolve(
    descriptor: &Descriptor, registry: &MountRegistry, location: &Location,
) -> Result<Workspace, Error> {
    let Descriptor::Dir(dir) = descriptor else {
        return Err(Error::Denied("grants.workspace root must be a directory".to_owned()));
    };

    // cap-std's `Metadata` derives the portable `(dev, ino)` the registry keys on
    let meta = Metadata::from_file(&dir.dir).map_err(|error| {
        Error::Other(format!("reading lent workspace directory metadata: {error}"))
    })?;
    let Some(entry) = registry.match_identity(meta.dev(), meta.ino()) else {
        return Err(Error::Denied("lent workspace root is not an authorized mount".to_owned()));
    };

    let subpath = &location.subpath;
    if subpath.is_empty() {
        return Ok(Workspace {
            dir: Arc::clone(&entry.dir),
            local_path: entry.host_path.clone(),
            writable: entry.writable,
        });
    }

    check_subpath(subpath)?;
    // cap-std's `open_dir` resolves beneath the verified mount root and
    // refuses any escape, so the subpath lend inherits the root's authority.
    let dir = entry.dir.open_dir(subpath).map_err(|error| match error.kind() {
        io::ErrorKind::NotFound => {
            Error::NotFound(format!("lent workspace subpath `{subpath}` does not exist"))
        }
        _ => Error::Other(format!("opening lent workspace subpath `{subpath}`: {error}")),
    })?;

    Ok(Workspace {
        dir: Arc::new(dir),
        local_path: entry.host_path.join(subpath),
        writable: entry.writable,
    })
}

// Refuse a subpath that is not a plain relative `/`-separated path. The
// component walk is what catches a Windows drive prefix such as `C:evil`,
// which is not a child name there.
fn check_subpath(subpath: &str) -> Result<(), Error> {
    let plain = !subpath.starts_with('/')
        && !subpath.contains('\\')
        && subpath.split('/').all(|part| !part.is_empty() && part != "." && part != "..")
        && Path::new(subpath).components().all(|part| matches!(part, Component::Normal(_)));
    if !plain {
        return Err(Error::Invalid(format!(
            "grants.workspace subpath `{subpath}` is not a plain relative path"
        )));
    }
    Ok(())
}
