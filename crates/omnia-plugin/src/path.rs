//! Compiled-in path acquisition over directories opened at construction.

use std::path::Path;
use std::sync::Arc;

use anyhow::{Context as _, Result};
use cap_std::ambient_authority;
use cap_std::fs::Dir;
use futures::FutureExt as _;
use futures::future::BoxFuture;

use crate::{Acquire, AcquireError, Location, MountEntry, read_entry};

/// Path acquisition over the composition root's own `(name, directory)`
/// entries, opened once at construction.
///
/// The `runtime!` macro's `locations:` path entries lower into it.
/// Resolution follows the guest's preopen rule (longest name prefix wins, a
/// bare relative path falls back to a `.` entry, plain relative subpaths
/// only), and every load reads fresh — never cached. Registry locations are
/// refused.
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

impl Acquire for PathAcquire {
    fn acquire<'a>(
        &'a self, package: &'a str, from: &'a Location,
    ) -> BoxFuture<'a, Result<Vec<u8>, AcquireError>> {
        async move {
            let Location::Path(path) = from else {
                return Err(AcquireError::Unsupported(format!(
                    "PathAcquire reads location-relative paths only; acquiring `{package}` \
                     from {from} requires a registry acquirer"
                )));
            };
            read_entry(path, &self.entries).await
        }
        .boxed()
    }
}
