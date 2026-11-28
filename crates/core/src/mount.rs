//! # Preopen mounts and the mount registry
//!
//! A deployment may mount node-local directories into a guest's WASI sandbox via
//! `[[mount]]` so a guest can read or edit a mounted directory tree. Two
//! host-side artefacts back that capability:
//!
//! - [`ResolvedPreopen`] — a mount as the deployment declares it: a host path
//!   plus writability, opened once into the [`MountRegistry`].
//! - [`MountRegistry`] — the host-side source of truth that maps a lent
//!   `wasi:filesystem` descriptor back to its mount by directory identity. It is
//!   built once at startup (opening each directory and capturing its identity),
//!   then shared read-only across every store. The registry — never the
//!   descriptor — supplies the resolved handle: a cap-std [`Dir`] for bounded
//!   operations and the canonical host path.
//!
//! Every store's `wasi:filesystem` preopens are the handles the registry
//! opened, so no store reopens a mount's path while guests run.
//!
//! A `wasi:filesystem` `Descriptor` is path-less by design (cap-std exposes no
//! API to recover a host path from a `Dir`), so the absolute path *must* come
//! from this registry, keyed by descriptor identity. Resolving a guest-lent
//! `borrow<descriptor>` into a usable handle is the job of the host crate that
//! defines that grant (e.g. `wasi-model`'s workspace tools), not of this crate.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context as _, Result, bail};
use cap_fs_ext::MetadataExt as _;
use cap_std::ambient_authority;
use cap_std::fs::Dir;
use wasmtime_wasi::{FsPerms, OpenMode};

/// A preopen mount resolved to a host path and writability, ready to be
/// opened into the guest sandbox and recorded in the [`MountRegistry`].
///
/// Writability maps to WASI filesystem permissions when the registry opens
/// the mount, keeping wasmtime types out of the mount API.
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
/// mount, plus the canonical `host_path` of the mount root. A lent descriptor
/// is matched back to this entry by directory identity (`identity`).
#[derive(Clone)]
pub struct Mount {
    /// Guest-visible name returned by `preopens.get-directories()`.
    pub name: String,
    /// Canonical host path of the mount root.
    pub host_path: PathBuf,
    /// Host-side capability handle to the mount root.
    pub dir: Arc<Dir>,
    /// Whether the mount permits writes; read-only otherwise.
    pub writable: bool,
    /// Directory identity `(device, inode)`, used to match a lent descriptor
    /// back to this entry.
    pub identity: (u64, u64),
    // what every store's preopens serve, one descriptor shared by them all
    pub(crate) preopen: wasmtime_wasi::filesystem::Dir,
}

/// The host-side registry of authorized mounts.
///
/// Built once at startup from the resolved preopens (opening each directory and
/// capturing its identity); shared read-only across every store via an `Arc`.
/// A consuming host crate matches a lent `borrow<descriptor>` against this
/// registry by directory identity, never trusting an OS path read out of the
/// descriptor.
#[derive(Clone, Default)]
pub struct MountRegistry {
    entries: Vec<Mount>,
}

impl MountRegistry {
    /// Open every resolved preopen and build the registry, the startup gate
    /// for a mount that cannot be served safely.
    ///
    /// - Each host path is canonicalised, so the path a mount reports passes
    ///   through no symlink a guest could replace.
    /// - Each directory is opened once. Every store preopens that handle, so
    ///   a mount stays the directory it was at startup whatever moves to its
    ///   path.
    /// - Preopens sharing a guest-visible name collapse to the last, so a CLI
    ///   `--mount` layered after a manifest `[[mount]]` overrides it rather
    ///   than being shadowed by wasi-libc's first-match resolution. The
    ///   discarded mount's host path is never opened.
    /// - A mount at or beneath another writable mount is refused, told by
    ///   directory identity, since a guest writing through the outer mount
    ///   could rearrange the inner one.
    ///
    /// # Errors
    ///
    /// Returns an error if a mount's host path cannot be resolved or opened
    /// as a directory, if its metadata cannot be read, or if it lies at or
    /// beneath another writable mount.
    pub fn open(preopens: Vec<ResolvedPreopen>) -> Result<Self> {
        let preopens = dedup_last_wins(preopens);
        let mut entries = Vec::with_capacity(preopens.len());
        for preopen in preopens {
            let host_path = std::fs::canonicalize(&preopen.host_path).with_context(|| {
                format!("resolving mount `{}` at {}", preopen.name, preopen.host_path.display())
            })?;
            let dir =
                Dir::open_ambient_dir(&host_path, ambient_authority()).with_context(|| {
                    format!("opening mount `{}` at {}", preopen.name, host_path.display())
                })?;
            let meta = dir
                .dir_metadata()
                .with_context(|| format!("reading metadata for mount `{}`", preopen.name))?;
            let preopen_dir = dir
                .try_clone()
                .with_context(|| format!("sharing mount `{}`", preopen.name))?
                .into_std_file();
            let (perms, mode) = if preopen.writable {
                (FsPerms::ReadWrite, OpenMode::READ | OpenMode::WRITE)
            } else {
                (FsPerms::ReadOnly, OpenMode::READ)
            };
            entries.push(Mount {
                name: preopen.name,
                host_path,
                dir: Arc::new(dir),
                writable: preopen.writable,
                identity: (meta.dev(), meta.ino()),
                preopen: wasmtime_wasi::filesystem::Dir::new(preopen_dir, perms, mode, false),
            });
        }

        let registry = Self { entries };
        registry.refuse_nested()?;
        Ok(registry)
    }

    // A guest writing through a writable mount can rename, remove, or link
    // anything beneath it, so it could rearrange another mount there for
    // whatever reads that mount by path. A mount is compared with the others
    // by position, so one sharing a writable mount's directory is refused too.
    fn refuse_nested(&self) -> Result<()> {
        for (index, mount) in self.entries.iter().enumerate() {
            let lineage = Self::ancestry(&mount.host_path).with_context(|| {
                format!("resolving mount `{}` at {}", mount.name, mount.host_path.display())
            })?;
            let outer = self.entries.iter().enumerate().find(|(other, outer)| {
                *other != index && outer.writable && lineage.contains(&outer.identity)
            });
            if let Some((_, outer)) = outer {
                let access = if mount.writable { "writable" } else { "read-only" };
                bail!(
                    "the {access} mount `{}` ({}) lies at or beneath the writable mount `{}` \
                     ({}): a guest writing through `{}` could rearrange it",
                    mount.name,
                    mount.host_path.display(),
                    outer.name,
                    outer.host_path.display(),
                    outer.name
                );
            }
        }
        Ok(())
    }

    /// The registered mounts, which every store serves as its WASI preopens.
    #[must_use]
    pub fn entries(&self) -> &[Mount] {
        &self.entries
    }

    // shares each kept mount's opened directory
    pub(crate) fn only(&self, keep: impl Fn(&Mount) -> bool) -> Self {
        Self {
            entries: self.entries.iter().filter(|mount| keep(mount)).cloned().collect(),
        }
    }

    /// The registered mount whose directory identity matches `(dev, ino)`, if
    /// any — how a lent descriptor selects its registry entry.
    #[must_use]
    pub fn match_identity(&self, dev: u64, ino: u64) -> Option<&Mount> {
        self.entries.iter().find(|entry| entry.identity == (dev, ino))
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
        Ok(self.beneath(path, |mount| mount.writable)?.map(|(mount, _)| mount))
    }

    /// The nearest mount `keep` admits that `path` lies at or beneath, with
    /// the rest of `path` below that mount's root.
    ///
    /// Told by directory identity, as
    /// [`beneath_writable`](Self::beneath_writable) is. A symlink in `path`
    /// is followed, so one that leads into another mount places `path` there.
    /// What lies below `path`'s nearest existing ancestor is kept as spelled.
    ///
    /// # Errors
    ///
    /// Returns an error if no ancestor of `path` exists, or one cannot be
    /// identified.
    pub fn beneath(
        &self, path: &Path, keep: impl Fn(&Mount) -> bool,
    ) -> Result<Option<(&Mount, PathBuf)>> {
        let lineage = Lineage::of(path)?;
        for (dir, identity) in &lineage.dirs {
            let found =
                self.entries.iter().find(|mount| mount.identity == *identity && keep(mount));
            if let Some(mount) = found {
                let rest = lineage.resolved.strip_prefix(dir).unwrap_or(&lineage.resolved);
                return Ok(Some((mount, rest.to_path_buf())));
            }
        }
        Ok(None)
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
        Ok(Lineage::of(path)?.dirs.into_iter().map(|(_, identity)| identity).collect())
    }
}

// Where a path stands: resolved through its nearest existing ancestor, with
// each directory from that ancestor up and its identity, nearest first.
struct Lineage {
    resolved: PathBuf,
    dirs: Vec<(PathBuf, (u64, u64))>,
}

impl Lineage {
    fn of(path: &Path) -> Result<Self> {
        let absolute = std::path::absolute(path)
            .with_context(|| format!("placing {} in the current directory", path.display()))?;
        let existing = absolute
            .ancestors()
            .find(|dir| dir.exists())
            .with_context(|| format!("no ancestor of {} exists", absolute.display()))?;
        let canonical =
            existing.canonicalize().with_context(|| format!("resolving {}", existing.display()))?;
        let dirs = canonical
            .ancestors()
            .map(|dir| {
                let meta = std::fs::metadata(dir)
                    .with_context(|| format!("identifying {}", dir.display()))?;
                Ok((dir.to_path_buf(), (meta.dev(), meta.ino())))
            })
            .collect::<Result<_>>()?;
        let below = absolute.strip_prefix(existing).unwrap_or_else(|_| Path::new(""));
        let resolved = below.components().fold(canonical, |mut resolved, part| {
            resolved.push(part);
            resolved
        });
        Ok(Self { resolved, dirs })
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
