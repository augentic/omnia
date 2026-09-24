//! Path acquisition over directories opened at construction.

use std::path::Path;
use std::sync::Arc;

use anyhow::{Context as _, Result, anyhow, ensure};
use cap_std::ambient_authority;
use cap_std::fs::Dir;
use futures::FutureExt as _;
use futures::future::BoxFuture;
use omnia_core::MountRegistry;

use crate::error::LoadError;
use crate::source::PathSource;

/// Path acquisition over named `(name, directory)` roots, resolved like guest
/// preopens and read fresh on every load.
///
/// The deployment's own mounts are the roots the declared policy installs
/// ([`From<&MountRegistry>`](Self::from)); [`new`](Self::new) opens roots of
/// a custom policy's choosing.
#[derive(Debug)]
pub struct PathMounts {
    entries: Vec<Mount>,
}

#[derive(Debug)]
struct Mount {
    name: String,
    dir: Arc<Dir>,
}

impl PathMounts {
    /// Opens every `(name, path)` entry now, surfacing a bad root as a
    /// configuration error before any load.
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
            let dir = Dir::open_ambient_dir(path, ambient_authority())
                .with_context(|| format!("opening load root `{name}` at {}", path.display()))?;

            opened.push(Mount {
                name,
                dir: Arc::new(dir),
            });
        }

        Ok(Self { entries: opened })
    }
}

// The mounts are already open: borrow their directory handles rather than
// opening each path a second time.
impl From<&MountRegistry> for PathMounts {
    fn from(mounts: &MountRegistry) -> Self {
        let entries = mounts
            .entries()
            .iter()
            .map(|mount| Mount {
                name: mount.name.clone(),
                dir: Arc::clone(&mount.dir),
            })
            .collect();
        Self { entries }
    }
}

impl PathSource for PathMounts {
    fn acquire<'a>(&'a self, path: &'a str) -> BoxFuture<'a, Result<Vec<u8>, LoadError>> {
        let entries = &self.entries;

        async move {
            let (dir, subpath) =
                resolve(path, entries).map_err(|error| LoadError::Refused(format!("{error:#}")))?;

            // file read is blocking I/O
            tokio::task::spawn_blocking(move || {
                dir.read(&subpath).with_context(|| format!("reading component `{subpath}`"))
            })
            .await
            .context("component read task panicked")
            .and_then(|res| res)
            .map_err(|err| LoadError::Unavailable(format!("{err:#}")))
        }
        .boxed()
    }
}

// Resolve `path` to a mount's capability handle plus the subpath within it,
// longest mount-name prefix first. The subpath must be plain and relative —
// cap-std then refuses any escape at open time.
fn resolve(path: &str, entries: &[Mount]) -> Result<(Arc<Dir>, String)> {
    let best = entries
        .iter()
        .filter_map(|entry| {
            if path == entry.name {
                // Naming a mount itself yields an empty subpath, which
                // `check_subpath` refuses — kept so the refusal names the
                // mount rather than "under no mount".
                return Some((entry, ""));
            }
            let subpath = path.strip_prefix(&entry.name)?.strip_prefix('/')?;
            Some((entry, subpath))
        })
        .max_by_key(|(entry, _)| entry.name.len())
        // wasi-libc resolves bare relative paths against a `.` preopen; do
        // the same so host- and guest-side views of a path agree.
        .or_else(|| entries.iter().find(|entry| entry.name == ".").map(|entry| (entry, path)));

    let (entry, subpath) =
        best.ok_or_else(|| anyhow!("path `{path}` is not under any mount of this deployment"))?;
    check_subpath(path, subpath)?;

    Ok((Arc::clone(&entry.dir), subpath.to_owned()))
}

fn check_subpath(path: &str, subpath: &str) -> Result<()> {
    // A leading '/' surfaces as an empty first segment, so the split check
    // also refuses absolute paths.
    let plain = !subpath.contains('\\')
        && subpath.split('/').all(|part| !part.is_empty() && part != "." && part != "..");
    ensure!(plain, "component path `{path}` is not a plain relative path within a mount");
    Ok(())
}
