//! # Preopen mounts and the mount registry
//!
//! A deployment may mount node-local directories into a guest's WASI sandbox via
//! `[[mount]]` so a guest can read or edit a mounted directory tree. Two
//! host-side artefacts back that capability:
//!
//! - [`ResolvedPreopen`] — a mount resolved to an absolute host path plus
//!   writability, applied per store via [`WasiCtxBuilder::preopened_dir`].
//! - [`MountRegistry`] — the host-side source of truth that maps a lent
//!   `wasi:filesystem` descriptor back to its mount by directory identity. It is
//!   built once at startup (opening each directory and capturing its identity),
//!   then shared read-only across every store. The registry — never the
//!   descriptor — supplies the resolved handle: a cap-std [`Dir`] for bounded
//!   operations and the absolute host path.
//!
//! A `wasi:filesystem` `Descriptor` is path-less by design (cap-std exposes no
//! API to recover a host path from a `Dir`), so the absolute path *must* come
//! from this registry, keyed by descriptor identity. Resolving a guest-lent
//! `borrow<descriptor>` into a usable handle is the job of the host crate that
//! defines that grant (e.g. `wasi-model`'s workspace tools), not of this crate.
//!
//! [`WasiCtxBuilder::preopened_dir`]: wasmtime_wasi::WasiCtxBuilder::preopened_dir

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context as _, Result};
use cap_fs_ext::MetadataExt as _;
use cap_std::ambient_authority;
use cap_std::fs::Dir;

/// A preopen mount resolved to an absolute host path and writability, ready
/// to be opened into the guest sandbox and recorded in the [`MountRegistry`].
///
/// Writability maps to WASI filesystem permissions only at the preopen call
/// site, keeping wasmtime types out of the mount API.
#[derive(Clone, Debug)]
pub struct ResolvedPreopen {
    /// Guest-visible name returned by `preopens.get-directories()`.
    pub name: String,
    /// Absolute host path of the mount root.
    pub host_path: PathBuf,
    /// Whether the mount permits writes; read-only otherwise.
    pub writable: bool,
}

impl ResolvedPreopen {
    /// Build a resolved preopen: read-only unless `writable`.
    #[must_use]
    pub const fn new(name: String, host_path: PathBuf, writable: bool) -> Self {
        Self {
            name,
            host_path,
            writable,
        }
    }
}

/// A single resolved mount: one record in the [`MountRegistry`].
///
/// Holds the cap-std [`Dir`] for bounded `read`/`list`/`write` confined to the
/// mount, plus the absolute `host_path` of the mount root. A lent descriptor is
/// matched back to this entry by directory identity (`identity`).
#[derive(Clone, Debug)]
pub struct Mount {
    /// Guest-visible name returned by `preopens.get-directories()`.
    pub name: String,
    /// Absolute host path of the mount root.
    pub host_path: PathBuf,
    /// Host-side capability handle to the mount root.
    pub dir: Arc<Dir>,
    /// Whether the mount permits writes; read-only otherwise.
    pub writable: bool,
    /// Directory identity `(device, inode)`, used to match a lent descriptor
    /// back to this entry.
    pub identity: (u64, u64),
}

/// The host-side registry of authorized mounts.
///
/// Built once at startup from the resolved preopens (opening each directory and
/// capturing its identity); cheaply cloned across stores via its internal
/// `Arc`. A consuming host crate matches a lent `borrow<descriptor>` against
/// this registry by directory identity, never trusting an OS path read out of
/// the descriptor.
#[derive(Clone, Debug, Default)]
pub struct MountRegistry {
    entries: Option<Arc<[Mount]>>,
}

impl MountRegistry {
    /// An empty mount registry; allocates nothing.
    #[must_use]
    pub const fn new() -> Self {
        Self { entries: None }
    }

    /// Open every resolved preopen, capturing its directory identity, and build
    /// the registry: the startup fail-fast gate for a mount whose host path
    /// cannot be opened as a directory.
    ///
    /// Preopens sharing a guest-visible name collapse to the last, so a CLI
    /// `--mount` layered after a manifest `[[mount]]` overrides it rather than
    /// being shadowed by wasi-libc's first-match resolution; the discarded
    /// mount's host path is never opened.
    ///
    /// # Errors
    ///
    /// Returns an error if a mount's host path cannot be opened as a directory
    /// or its metadata cannot be read.
    pub fn open(preopens: Vec<ResolvedPreopen>) -> Result<Self> {
        let preopens = dedup_last_wins(preopens);
        let mut entries = Vec::with_capacity(preopens.len());
        for preopen in preopens {
            let dir = Dir::open_ambient_dir(&preopen.host_path, ambient_authority()).with_context(
                || format!("opening mount `{}` at {}", preopen.name, preopen.host_path.display()),
            )?;
            let meta = dir
                .dir_metadata()
                .with_context(|| format!("reading metadata for mount `{}`", preopen.name))?;
            entries.push(Mount {
                name: preopen.name,
                host_path: preopen.host_path,
                dir: Arc::new(dir),
                writable: preopen.writable,
                identity: (meta.dev(), meta.ino()),
            });
        }
        Ok(Self {
            entries: (!entries.is_empty()).then(|| Arc::from(entries)),
        })
    }

    /// The registered mounts — the WASI preopens to apply to each store.
    #[must_use]
    pub fn entries(&self) -> &[Mount] {
        self.entries.as_deref().unwrap_or(&[])
    }

    /// The registered mount whose directory identity matches `(dev, ino)`, if
    /// any — how a lent descriptor selects its registry entry.
    #[must_use]
    pub fn match_identity(&self, dev: u64, ino: u64) -> Option<&Mount> {
        self.entries().iter().find(|entry| entry.identity == (dev, ino))
    }

    /// The writable mount `path` lies at or beneath, if any — told by
    /// directory identity, not by spelling, so a second path to one
    /// directory is the same directory here.
    ///
    /// A path that does not exist yet is placed by its nearest existing
    /// ancestor, so a directory a guest could create through a writable
    /// mount is beneath it before it is there.
    ///
    /// # Errors
    ///
    /// Returns an error if no ancestor of `path` exists, or one cannot be
    /// identified.
    pub fn beneath_writable(&self, path: &Path) -> Result<Option<&Mount>> {
        let lineage = Self::ancestry(path)?;
        Ok(self.entries().iter().find(|mount| mount.writable && lineage.contains(&mount.identity)))
    }

    /// The `(device, inode)` identity of `path`'s nearest existing ancestor
    /// (`path` itself when it exists) and of every directory above it,
    /// nearest first. A relative `path` is placed against the current
    /// directory.
    ///
    /// # Errors
    ///
    /// Returns an error if no ancestor of `path` exists, or one cannot be
    /// identified.
    pub fn ancestry(path: &Path) -> Result<Vec<(u64, u64)>> {
        let absolute = std::path::absolute(path)
            .with_context(|| format!("placing {} in the current directory", path.display()))?;
        let existing = absolute
            .ancestors()
            .find(|dir| dir.exists())
            .with_context(|| format!("no ancestor of {} exists", absolute.display()))?;
        let canonical =
            existing.canonicalize().with_context(|| format!("resolving {}", existing.display()))?;
        canonical
            .ancestors()
            .map(|dir| {
                let meta = std::fs::metadata(dir)
                    .with_context(|| format!("identifying {}", dir.display()))?;
                Ok((meta.dev(), meta.ino()))
            })
            .collect()
    }
}

// The winner keeps the position of its name's first occurrence, so the order
// is stable; only the value comes from the last.
fn dedup_last_wins(preopens: Vec<ResolvedPreopen>) -> Vec<ResolvedPreopen> {
    let mut deduped: Vec<ResolvedPreopen> = Vec::with_capacity(preopens.len());
    let mut index: HashMap<String, usize> = HashMap::new();
    for preopen in preopens {
        if let Some(&position) = index.get(&preopen.name) {
            deduped[position] = preopen;
        } else {
            index.insert(preopen.name.clone(), deduped.len());
            deduped.push(preopen);
        }
    }
    deduped
}
