//! Compiled-in path acquisition over directories opened at construction.

use std::path::Path;
use std::sync::Arc;

use anyhow::{Context as _, Result, anyhow, ensure};
use cap_std::ambient_authority;
use cap_std::fs::Dir;
use futures::FutureExt as _;
use futures::future::BoxFuture;

use crate::AcquirePath;

/// Path acquisition over the composition root's own `(name, directory)`
/// entries, opened once at construction — the [`Acquirer::path`] slot the
/// `runtime!` macro's `locations:` path entries lower into.
///
/// Resolution follows the guest's preopen rule (longest name prefix wins, a
/// bare relative path falls back to a `.` entry, plain relative subpaths
/// only), and every load reads fresh — never cached.
///
/// [`Acquirer::path`]: crate::Acquirer::path
#[derive(Debug)]
pub struct PathAcquire {
    entries: Vec<MountEntry>,
}

#[derive(Clone, Debug)]
struct MountEntry {
    name: String,
    dir: Arc<cap_std::fs::Dir>,
}

impl PathAcquire {
    /// Open every `(name, path)` entry now — the startup fail-fast gate: a
    /// location whose path cannot be opened as a directory is a
    /// configuration error surfaced before any load.
    ///
    /// # Errors
    ///
    /// Returns an error if a path cannot be opened as a directory.
    pub fn new<N, P>(entries: impl IntoIterator<Item = (N, P)>) -> Result<Self>
    where
        N: Into<String>,
        P: AsRef<Path>,
    {
        let mut opened = Vec::new();
        for (name, path) in entries {
            let name = name.into();
            let path = path.as_ref();
            let dir = Dir::open_ambient_dir(path, ambient_authority()).with_context(|| {
                format!("opening plugins location `{name}` at {}", path.display())
            })?;
            opened.push(MountEntry {
                name,
                dir: Arc::new(dir),
            });
        }
        Ok(Self { entries: opened })
    }
}

impl AcquirePath for PathAcquire {
    fn acquire<'a>(&'a self, path: &'a str) -> BoxFuture<'a, Result<Vec<u8>>> {
        read_entry(path, &self.entries).boxed()
    }
}

async fn read_entry(path: &str, entries: &[MountEntry]) -> Result<Vec<u8>> {
    let (dir, subpath) = resolve(path, entries)?;
    // File reads are blocking I/O; keep them off the async executor.
    tokio::task::spawn_blocking(move || {
        dir.read(&subpath).with_context(|| format!("reading component `{subpath}`"))
    })
    .await
    .context("component read task panicked")?
}

// Resolve `path` to a location's capability handle plus the subpath within it.
// The longest location-name prefix wins; a plain relative path with no
// matching prefix falls back to a location named `.` when one exists. The
// subpath must be plain and relative — cap-std then refuses any escape at
// open time.
fn resolve(path: &str, entries: &[MountEntry]) -> Result<(Arc<cap_std::fs::Dir>, String)> {
    let mut best: Option<(&MountEntry, &str)> = None;
    for entry in entries {
        let subpath = if path == entry.name {
            ""
        } else if let Some(rest) = path.strip_prefix(&entry.name).and_then(|r| r.strip_prefix('/'))
        {
            rest
        } else {
            continue;
        };
        if best.is_none_or(|(current, _)| entry.name.len() > current.name.len()) {
            best = Some((entry, subpath));
        }
    }
    
    if best.is_none() {
        // wasi-libc resolves bare relative paths against a `.` preopen; do
        // the same so host- and guest-side views of a path agree.
        best = entries.iter().find(|entry| entry.name == ".").map(|entry| (entry, path));
    }
    let (entry, subpath) =
        best.ok_or_else(|| anyhow!("path `{path}` is not under any location of this deployment"))?;
    check_subpath(path, subpath)?;

    Ok((Arc::clone(&entry.dir), subpath.to_owned()))
}

// Refuse a subpath that is not a plain relative `/`-separated path.
fn check_subpath(path: &str, subpath: &str) -> Result<()> {
    let plain = !subpath.starts_with('/')
        && !subpath.contains('\\')
        && subpath.split('/').all(|part| !part.is_empty() && part != "." && part != "..");
    ensure!(plain, "component path `{path}` is not a plain relative path within a location");
    Ok(())
}
