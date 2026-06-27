//! # Working-tree preopens and registry (RFC-55)
//!
//! A deployment may mount node-local directories into a guest's WASI sandbox so
//! a model completion can read or edit a working tree. Two host-side artefacts
//! back that capability:
//!
//! - [`ResolvedPreopen`] — a mount resolved to an absolute host path plus WASI
//!   permissions, applied per store via [`WasiCtxBuilder::preopened_dir`].
//! - [`WorkingTreeRegistry`] — the host-side source of truth that maps a lent
//!   `wasi:filesystem` descriptor back to its mount by directory identity. It is
//!   built once at startup (opening each directory and capturing its identity),
//!   then shared read-only across every store. The registry — never the
//!   descriptor — supplies the resolved faces: a cap-std [`Dir`] for bounded
//!   operations (genai) and the absolute host path (cursor).
//!
//! A `wasi:filesystem` `Descriptor` is path-less by design (cap-std exposes no
//! API to recover a host path from a `Dir`), so the absolute path *must* come
//! from this registry, keyed by descriptor identity, never extracted from the
//! descriptor itself.
//!
//! [`WasiCtxBuilder::preopened_dir`]: wasmtime_wasi::WasiCtxBuilder::preopened_dir

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context as _, Result};
use cap_fs_ext::MetadataExt as _;
use cap_std::ambient_authority;
use cap_std::fs::Dir;
use wasmtime_wasi::{DirPerms, FilePerms};

/// A preopen mount resolved to an absolute host path and WASI permissions,
/// ready to be opened into the guest sandbox and recorded in the
/// [`WorkingTreeRegistry`].
#[derive(Clone, Debug)]
pub struct ResolvedPreopen {
    /// Guest-visible name returned by `preopens.get-directories()`.
    pub name: String,
    /// Absolute host path of the mount root.
    pub host_path: PathBuf,
    /// Directory permissions WASI enforces on the mount.
    pub dir_perms: DirPerms,
    /// File permissions WASI enforces on files opened under the mount.
    pub file_perms: FilePerms,
}

impl ResolvedPreopen {
    /// Build a resolved preopen, deriving WASI permissions from `writable`:
    /// read-only for review flows, read+write for agent edit flows.
    #[must_use]
    pub const fn new(name: String, host_path: PathBuf, writable: bool) -> Self {
        let (dir_perms, file_perms) = if writable {
            (DirPerms::all(), FilePerms::all())
        } else {
            (DirPerms::READ, FilePerms::READ)
        };
        Self {
            name,
            host_path,
            dir_perms,
            file_perms,
        }
    }
}

/// One resolved working-tree mount: the registry's record for both faces.
///
/// Holds the cap-std [`Dir`] for genai's bounded `read`/`list`/`write` and the
/// absolute `host_path` for cursor's `--workspace`. A lent descriptor is matched
/// back to this entry by directory identity (`identity`).
#[derive(Clone)]
pub struct WorkingTreeEntry {
    /// Guest-visible name returned by `preopens.get-directories()`.
    pub name: String,
    /// Absolute host path of the mount root (the `local-path` face).
    pub host_path: PathBuf,
    /// Host-side capability handle to the mount root (the `descriptor` face).
    pub dir: Arc<Dir>,
    /// Directory permissions configured for the mount.
    pub dir_perms: DirPerms,
    /// File permissions configured for the mount.
    pub file_perms: FilePerms,
    /// Directory identity `(device, inode)`, used to match a lent descriptor
    /// back to this entry.
    pub identity: (u64, u64),
}

impl WorkingTreeEntry {
    /// Whether the mount permits writes (read+write), derived from `dir_perms`.
    #[must_use]
    pub const fn writable(&self) -> bool {
        self.dir_perms.contains(DirPerms::MUTATE)
    }
}

/// The host-side registry of authorized working-tree mounts.
///
/// Built once at startup from the resolved preopens (opening each directory and
/// capturing its identity); shared read-only across every store via an `Arc`.
/// The floor matches a lent `borrow<descriptor>` against this registry by
/// directory identity, never trusting an OS path read out of the descriptor.
#[derive(Clone, Default)]
pub struct WorkingTreeRegistry {
    entries: Vec<WorkingTreeEntry>,
}

impl WorkingTreeRegistry {
    /// Open every resolved preopen, capturing its directory identity, and build
    /// the registry. This is the startup fail-fast gate: a mount whose host
    /// path cannot be opened as a directory (or stat-ed) is a configuration
    /// error surfaced before the registry is built.
    ///
    /// # Errors
    ///
    /// Returns an error if a mount's host path cannot be opened as a directory
    /// or its metadata cannot be read.
    pub fn open(preopens: Vec<ResolvedPreopen>) -> Result<Self> {
        let mut entries = Vec::with_capacity(preopens.len());
        for preopen in preopens {
            let dir = Dir::open_ambient_dir(&preopen.host_path, ambient_authority()).with_context(
                || {
                    format!(
                        "opening working-tree mount `{}` at {}",
                        preopen.name,
                        preopen.host_path.display()
                    )
                },
            )?;
            let meta = dir.dir_metadata().with_context(|| {
                format!("reading metadata for working-tree mount `{}`", preopen.name)
            })?;
            entries.push(WorkingTreeEntry {
                name: preopen.name,
                host_path: preopen.host_path,
                dir: Arc::new(dir),
                dir_perms: preopen.dir_perms,
                file_perms: preopen.file_perms,
                identity: (meta.dev(), meta.ino()),
            });
        }
        Ok(Self { entries })
    }

    /// The registered mounts — the WASI preopens to apply to each store.
    #[must_use]
    pub fn entries(&self) -> &[WorkingTreeEntry] {
        &self.entries
    }

    /// Whether the registry holds no mounts.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// The registered mount whose directory identity matches `(dev, ino)`, if
    /// any — how a lent descriptor selects its registry entry.
    #[must_use]
    pub fn match_identity(&self, dev: u64, ino: u64) -> Option<&WorkingTreeEntry> {
        self.entries.iter().find(|entry| entry.identity == (dev, ino))
    }
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use cap_fs_ext::MetadataExt as _;
    use cap_std::ambient_authority;
    use cap_std::fs::Dir;
    use wasmtime_wasi::DirPerms;

    use super::{ResolvedPreopen, WorkingTreeRegistry};

    /// A fresh, empty temp directory unique to this process and `label`.
    fn temp_root(label: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("omnia-reg-{label}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("creating temp mount root");
        dir
    }

    /// `(dev, ino)` of `path`, computed independently of the registry under test.
    fn identity_of(path: &Path) -> (u64, u64) {
        let dir = Dir::open_ambient_dir(path, ambient_authority()).expect("opening dir");
        let meta = dir.dir_metadata().expect("reading metadata");
        (meta.dev(), meta.ino())
    }

    fn registry(name: &str, path: &Path, writable: bool) -> WorkingTreeRegistry {
        WorkingTreeRegistry::open(vec![ResolvedPreopen::new(
            name.to_owned(),
            path.to_path_buf(),
            writable,
        )])
        .expect("opening registry")
    }

    #[test]
    fn open_records_identity_and_path() {
        let root = temp_root("identity");
        let registry = registry(".", &root, false);

        assert!(!registry.is_empty());
        let entry = &registry.entries()[0];
        assert_eq!(entry.name, ".");
        assert_eq!(entry.host_path, root);
        assert!(!entry.writable(), "a mount defaults to read-only");
        assert_eq!(entry.identity, identity_of(&root), "the entry records the dir's (dev, ino)");
    }

    #[test]
    fn match_identity_selects_by_dev_ino() {
        let root = temp_root("select");
        let registry = registry(".", &root, false);
        let (dev, ino) = identity_of(&root);

        let hit = registry.match_identity(dev, ino).expect("the mount's identity matches");
        assert_eq!(hit.host_path, root);
        // A foreign identity matches nothing — the out-of-scope rejection the
        // floor relies on.
        assert!(registry.match_identity(dev ^ 0xFFFF, ino ^ 0xFFFF).is_none());
    }

    #[test]
    fn writable_mount_grants_mutate() {
        let root = temp_root("writable");
        let registry = registry("tree", &root, true);

        let entry = &registry.entries()[0];
        assert!(entry.writable(), "a writable mount permits mutation");
        assert!(entry.dir_perms.contains(DirPerms::MUTATE));
    }
}
