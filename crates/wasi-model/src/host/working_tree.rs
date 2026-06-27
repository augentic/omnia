//! Working-tree resolution at the floor (RFC-55, Phase 2b).
//!
//! A guest lends a `wasi:filesystem` working tree through
//! `grants.working-tree: option<borrow<descriptor>>`. This module turns that
//! borrowed descriptor into an owned, `Send + Sync` [`WorkingTree`] the backend
//! can use across `.await` points, *after* proving the lent directory is one the
//! deployment authorized.
//!
//! The proof is identity, not paths. A `wasi:filesystem` descriptor is path-less
//! by design, and a guest must never be trusted to name its own scope, so the
//! floor stats the lent directory for its `(device, inode)` identity and matches
//! it against the host-side [`WorkingTreeRegistry`] (built from the deployment's
//! preopens). A miss — a sub-directory of a mount, or a wholly unrelated tree —
//! is rejected here, at the floor. The resolved [`WorkingTree`] then draws its
//! cap-std handle and absolute path from the *registry entry*, never from the
//! descriptor.

use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context as _, bail};
use cap_fs_ext::MetadataExt as _;
use cap_std::fs::Dir;
use futures::FutureExt as _;
use omnia::{FutureResult, WorkingTreeRegistry};
use wasmtime::component::Resource;
use wasmtime_wasi::ResourceTable;
use wasmtime_wasi::filesystem::Descriptor;

use super::types::DirEntry;

/// Maximum bytes a single [`WorkingTree::read`] returns; larger files are
/// rejected rather than truncated, so the model never sees a silently clipped
/// file.
const MAX_READ_BYTES: u64 = 4 * 1024 * 1024;

/// Maximum bytes a single [`WorkingTree::write`] accepts.
const MAX_WRITE_BYTES: usize = 4 * 1024 * 1024;

/// Maximum entries a single [`WorkingTree::list`] returns; a larger directory is
/// rejected so the caller narrows the path rather than receiving a partial list.
const MAX_LIST_ENTRIES: usize = 4096;

/// An owned, `Send + Sync` handle to a resolved working-tree mount.
///
/// Built by [`resolve_working_tree`] once a lent descriptor has identity-matched
/// an authorized [`WorkingTreeRegistry`] entry. It holds the registry's cap-std
/// [`Dir`] — so bounded `read`/`list`/`write` ride the same sandbox WASI itself
/// enforces — plus the mount's absolute host path (the `local-path` face cursor
/// consumes) and whether writes are permitted.
pub struct WorkingTree {
    dir: Arc<Dir>,
    local_path: PathBuf,
    writable: bool,
}

impl WorkingTree {
    /// The mount's absolute host path — the `local-path` face (RFC-55), e.g.
    /// cursor's `--workspace`.
    #[must_use]
    pub fn local_path(&self) -> &Path {
        &self.local_path
    }

    /// Bounded read of `path` relative to the mount root, capped at
    /// [`MAX_READ_BYTES`]. cap-std rejects absolute paths and `..` escapes, so
    /// the read can never leave the mount.
    pub fn read(&self, path: String) -> FutureResult<Vec<u8>> {
        let dir = Arc::clone(&self.dir);
        async move {
            tokio::task::spawn_blocking(move || read_blocking(&dir, &path))
                .await
                .context("working-tree read task failed")?
        }
        .boxed()
    }

    /// Bounded listing of `path` relative to the mount root (an empty path or
    /// `.` lists the root), capped at [`MAX_LIST_ENTRIES`]. Returns entry names
    /// only — never OS paths.
    pub fn list(&self, path: String) -> FutureResult<Vec<DirEntry>> {
        let dir = Arc::clone(&self.dir);
        async move {
            tokio::task::spawn_blocking(move || list_blocking(&dir, &path))
                .await
                .context("working-tree list task failed")?
        }
        .boxed()
    }

    /// Write `bytes` to `path` relative to the mount root, capped at
    /// [`MAX_WRITE_BYTES`]. Denied on a read-only mount; cap-std rejects absolute
    /// paths and `..` escapes.
    pub fn write(&self, path: String, bytes: Vec<u8>) -> FutureResult<()> {
        if !self.writable {
            return async move {
                Err(anyhow::anyhow!("working tree is read-only; write to `{path}` denied"))
            }
            .boxed();
        }
        let dir = Arc::clone(&self.dir);
        async move {
            tokio::task::spawn_blocking(move || write_blocking(&dir, &path, &bytes))
                .await
                .context("working-tree write task failed")?
        }
        .boxed()
    }
}

/// Resolve a lent `grants.working-tree` borrow into an owned [`WorkingTree`].
///
/// `None` grant resolves to `Ok(None)`. A present borrow must (1) resolve in the
/// resource table, (2) be a *directory* descriptor (a file is rejected), and
/// (3) identity-match (`(dev, ino)`) an authorized [`WorkingTreeRegistry`] entry.
/// A miss — an out-of-scope or unauthorized tree — is an error raised here, at
/// the floor, with no ambient fallback.
///
/// # Errors
///
/// Returns an error if the borrow does not resolve, is not a directory, its
/// metadata cannot be read, or it matches no authorized mount.
pub fn resolve_working_tree(
    table: &ResourceTable, registry: &WorkingTreeRegistry, borrow: Option<&Resource<Descriptor>>,
) -> anyhow::Result<Option<WorkingTree>> {
    let Some(resource) = borrow else {
        return Ok(None);
    };

    // The lent capability is a `borrow<descriptor>`; resolve it from the table
    // exactly as the filesystem host does — never trusting a forgeable handle.
    let descriptor = table.get(resource).context("resolving the lent working-tree descriptor")?;

    // A working tree is a directory; a file descriptor is never a tree.
    let Descriptor::Dir(dir) = descriptor else {
        bail!("grants.working-tree must be a directory descriptor, not a file");
    };

    // Identity-stamp the lent directory and match it against the authorized
    // registry. Only the `(dev, ino)` identity is trusted — never an OS path
    // read out of the descriptor, which the guest could otherwise forge.
    let meta = dir.dir.dir_metadata().context("reading lent working-tree directory metadata")?;
    let entry = registry
        .match_identity(meta.dev(), meta.ino())
        .context("lent working tree is not an authorized mount (out of scope)")?;

    Ok(Some(WorkingTree {
        dir: Arc::clone(&entry.dir),
        local_path: entry.host_path.clone(),
        writable: entry.writable(),
    }))
}

/// Bounded blocking read, run on the blocking pool.
fn read_blocking(dir: &Dir, path: &str) -> anyhow::Result<Vec<u8>> {
    let file = dir.open(path).with_context(|| format!("opening `{path}` in working tree"))?;
    // Read one byte past the cap so an over-limit file is detected, not clipped.
    let mut buf = Vec::new();
    file.take(MAX_READ_BYTES + 1)
        .read_to_end(&mut buf)
        .with_context(|| format!("reading `{path}` in working tree"))?;
    if buf.len() as u64 > MAX_READ_BYTES {
        bail!("file `{path}` exceeds the {MAX_READ_BYTES}-byte working-tree read limit");
    }
    Ok(buf)
}

/// Bounded blocking directory listing, run on the blocking pool.
fn list_blocking(dir: &Dir, path: &str) -> anyhow::Result<Vec<DirEntry>> {
    let read_dir = if path.is_empty() || path == "." {
        dir.entries().context("listing working-tree root")?
    } else {
        dir.read_dir(path).with_context(|| format!("listing `{path}` in working tree"))?
    };

    let mut entries = Vec::new();
    for entry in read_dir {
        let entry = entry.context("reading working-tree directory entry")?;
        if entries.len() >= MAX_LIST_ENTRIES {
            bail!("directory `{path}` exceeds the {MAX_LIST_ENTRIES}-entry listing limit");
        }
        // A failed `file_type` (e.g. a vanished entry) defaults to non-directory
        // rather than failing the whole listing.
        let is_directory = entry.file_type().is_ok_and(|file_type| file_type.is_dir());
        entries.push(DirEntry {
            name: entry.file_name().to_string_lossy().into_owned(),
            is_directory,
        });
    }
    Ok(entries)
}

/// Bounded blocking write, run on the blocking pool.
fn write_blocking(dir: &Dir, path: &str, bytes: &[u8]) -> anyhow::Result<()> {
    if bytes.len() > MAX_WRITE_BYTES {
        bail!("write to `{path}` exceeds the {MAX_WRITE_BYTES}-byte working-tree write limit");
    }
    dir.write(path, bytes).with_context(|| format!("writing `{path}` in working tree"))
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use cap_std::ambient_authority;
    use cap_std::fs::Dir as CapDir;
    use omnia::{ResolvedPreopen, WorkingTreeRegistry};
    use wasmtime_wasi::filesystem::{Descriptor, Dir, File, OpenMode};
    use wasmtime_wasi::{DirPerms, FilePerms, ResourceTable};

    use super::resolve_working_tree;

    /// A fresh, empty temp directory unique to this process and `label`.
    fn temp_root(label: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("omnia-wt-{label}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("creating temp working-tree root");
        dir
    }

    /// A single-mount registry named `.` over `path`, mirroring how a `[[mount]]`
    /// resolves (read-only unless `writable`).
    fn registry_for(path: &Path, writable: bool) -> WorkingTreeRegistry {
        WorkingTreeRegistry::open(vec![ResolvedPreopen::new(
            ".".to_owned(),
            path.to_path_buf(),
            writable,
        )])
        .expect("opening working-tree registry")
    }

    /// A directory `Descriptor` over `path`, the shape a guest lends through
    /// `grants.working-tree`.
    fn dir_descriptor(path: &Path, writable: bool) -> Descriptor {
        let cap = CapDir::open_ambient_dir(path, ambient_authority()).expect("opening ambient dir");
        let (dir_perms, file_perms, mode) = if writable {
            (DirPerms::all(), FilePerms::all(), OpenMode::READ | OpenMode::WRITE)
        } else {
            (DirPerms::READ, FilePerms::READ, OpenMode::READ)
        };
        Descriptor::Dir(Dir::new(cap, dir_perms, file_perms, mode, false))
    }

    #[test]
    fn no_grant_resolves_to_none() {
        let table = ResourceTable::new();
        let registry = WorkingTreeRegistry::default();
        let resolved = resolve_working_tree(&table, &registry, None).expect("resolve");
        assert!(resolved.is_none(), "an absent grant resolves to None");
    }

    #[test]
    fn authorized_directory_identity_matches() {
        let root = temp_root("match");
        let registry = registry_for(&root, false);

        let mut table = ResourceTable::new();
        let resource = table.push(dir_descriptor(&root, false)).expect("pushing descriptor");

        let resolved = resolve_working_tree(&table, &registry, Some(&resource))
            .expect("resolve succeeds for an authorized mount")
            .expect("an authorized mount resolves to a working tree");
        assert_eq!(
            resolved.local_path(),
            root.as_path(),
            "local_path is the registry mount's host path, never read from the descriptor"
        );
    }

    #[tokio::test]
    async fn resolved_tree_reads_within_the_mount() {
        let root = temp_root("read");
        std::fs::write(root.join("hello.txt"), b"hi").expect("seeding a file");
        let registry = registry_for(&root, false);
        let mut table = ResourceTable::new();
        let resource = table.push(dir_descriptor(&root, false)).expect("pushing descriptor");

        let tree = resolve_working_tree(&table, &registry, Some(&resource))
            .expect("resolve")
            .expect("authorized mount");
        let bytes = tree.read("hello.txt".to_owned()).await.expect("reading within the mount");
        assert_eq!(bytes, b"hi", "read returns the file's bytes");
    }

    #[test]
    fn out_of_scope_directory_is_rejected() {
        let authorized = temp_root("scope-ok");
        let other = temp_root("scope-bad");
        let registry = registry_for(&authorized, false);

        let mut table = ResourceTable::new();
        // A directory that is *not* an authorized mount (a sibling tree).
        let resource = table.push(dir_descriptor(&other, false)).expect("pushing descriptor");

        let Err(err) = resolve_working_tree(&table, &registry, Some(&resource)) else {
            panic!("an unauthorized tree must be rejected at the floor");
        };
        assert!(
            format!("{err:#}").contains("out of scope"),
            "rejection names the out-of-scope cause: {err:#}"
        );
    }

    #[test]
    fn file_descriptor_is_rejected() {
        let root = temp_root("nondir");
        std::fs::write(root.join("not-a-dir.txt"), b"x").expect("seeding a file");
        let registry = registry_for(&root, false);

        let cap_dir = CapDir::open_ambient_dir(&root, ambient_authority()).expect("opening dir");
        let cap_file = cap_dir.open("not-a-dir.txt").expect("opening file");
        let descriptor =
            Descriptor::File(File::new(cap_file, FilePerms::READ, OpenMode::READ, false));
        let mut table = ResourceTable::new();
        let resource = table.push(descriptor).expect("pushing descriptor");

        let Err(err) = resolve_working_tree(&table, &registry, Some(&resource)) else {
            panic!("a file descriptor must not resolve to a working tree");
        };
        assert!(
            format!("{err:#}").contains("must be a directory"),
            "rejection names the non-directory cause: {err:#}"
        );
    }

    #[tokio::test]
    async fn write_to_read_only_mount_is_denied() {
        let root = temp_root("ro");
        let registry = registry_for(&root, false);
        let mut table = ResourceTable::new();
        let resource = table.push(dir_descriptor(&root, false)).expect("pushing descriptor");

        let tree = resolve_working_tree(&table, &registry, Some(&resource))
            .expect("resolve")
            .expect("authorized mount");
        let err = tree
            .write("new.txt".to_owned(), b"data".to_vec())
            .await
            .expect_err("writing a read-only mount is denied");
        assert!(
            format!("{err:#}").contains("read-only"),
            "the denial names the read-only mount: {err:#}"
        );
        assert!(!root.join("new.txt").exists(), "no file is created on a denied write");
    }
}
