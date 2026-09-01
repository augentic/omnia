//! Path acquisition over directories opened at construction.

use std::path::Path;
use std::sync::Arc;

use anyhow::{Context as _, Result, anyhow, ensure};
use cap_std::ambient_authority;
use cap_std::fs::Dir;
use futures::FutureExt as _;
use futures::future::BoxFuture;

use crate::LoadError;

/// Path acquisition policy — the path slot of [`Plugins`](crate::Plugins).
pub trait PathSource: Send + Sync + 'static {
    /// Produce the raw component bytes at the location-relative `path`,
    /// split by remedy: [`LoadError::Refused`] for a path no location
    /// serves, never for a read failure a retry might clear
    /// ([`LoadError::Unavailable`]).
    fn acquire<'a>(&'a self, path: &'a str) -> BoxFuture<'a, Result<Vec<u8>, LoadError>>;
}

/// Path acquisition over named `(name, directory)` roots, resolved like guest
/// preopens and read fresh on every load.
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
    /// Opens every `(name, path)` entry now, surfacing a bad location as a
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
            let dir = Dir::open_ambient_dir(path, ambient_authority()).with_context(|| {
                format!("opening plugins location `{name}` at {}", path.display())
            })?;

            opened.push(Mount {
                name,
                dir: Arc::new(dir),
            });
        }

        Ok(Self { entries: opened })
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

// Resolve `path` to a location's capability handle plus the subpath within
// it, longest location-name prefix first. The subpath must be plain and
// relative — cap-std then refuses any escape at open time.
fn resolve(path: &str, entries: &[Mount]) -> Result<(Arc<Dir>, String)> {
    let mut best: Option<(&Mount, &str)> = None;
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

fn check_subpath(path: &str, subpath: &str) -> Result<()> {
    let plain = !subpath.starts_with('/')
        && !subpath.contains('\\')
        && subpath.split('/').all(|part| !part.is_empty() && part != "." && part != "..");
    ensure!(plain, "component path `{path}` is not a plain relative path within a location");
    Ok(())
}
