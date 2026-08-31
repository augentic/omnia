//! Compiled-in path acquisition over directories opened at construction.

use std::path::Path;
use std::sync::Arc;

use anyhow::{Context as _, Result};
use cap_std::ambient_authority;
use cap_std::fs::Dir;
use futures::FutureExt as _;
use futures::future::BoxFuture;

use crate::{AcquirePath, MountEntry, read_entry};

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
