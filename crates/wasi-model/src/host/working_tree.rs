//! Working-tree resolution in the host.
//!
//! A guest lends a `wasi:filesystem` working tree through
//! `grants.working-tree: option<borrow<descriptor>>`. This module turns that
//! borrowed descriptor into an owned, `Send + Sync` [`WorkingTree`] the backend
//! can use across `.await` points, *after* proving the lent directory is one the
//! deployment authorized.
//!
//! The proof is identity, not paths. A `wasi:filesystem` descriptor is path-less
//! by design, and a guest must never be trusted to name its own scope, so the
//! host platform stats the lent directory for its `(device, inode)` identity and matches
//! it against the host-side [`MountRegistry`] (built from the deployment's
//! preopens). A miss — a sub-directory of a mount, or a wholly unrelated tree —
//! is rejected here, in the host. The resolved [`WorkingTree`] then draws its
//! cap-std handle and absolute path from the *registry entry*, never from the
//! descriptor.

use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context as _, anyhow, bail};
use cap_fs_ext::MetadataExt as _;
use cap_std::fs::Dir;
use futures::FutureExt as _;
use omnia::{FutureResult, MountRegistry};
use tokio::task::spawn_blocking;
use wasmtime::component::{Resource, ResourceTable};
use wasmtime_wasi::filesystem::Descriptor;

use super::types::DirEntry;

const MAX_READ_BYTES: u64 = 4 * 1024 * 1024;
const MAX_WRITE_BYTES: usize = 4 * 1024 * 1024;
const MAX_LIST_ENTRIES: usize = 4096;

// An handle to a resolved working-tree mount. Built by [`resolve`].
pub struct WorkingTree {
    dir: Arc<Dir>,
    local_path: PathBuf,
    writable: bool,
}

impl WorkingTree {
    #[must_use]
    pub fn local_path(&self) -> &Path {
        &self.local_path
    }

    // Off-thread a bounded, blocking cap-std op against the mount, tagging a
    // task-join failure with `op`.
    fn run_blocking<R: Send + 'static>(
        &self, op: &'static str, f: impl FnOnce(&Dir) -> anyhow::Result<R> + Send + 'static,
    ) -> FutureResult<R> {
        let dir = Arc::clone(&self.dir);
        async move {
            spawn_blocking(move || f(&dir))
                .await
                .context("working-tree read task failed")?
        }
        .boxed()
    }

    pub fn read(&self, path: String) -> FutureResult<Vec<u8>> {
        self.run_blocking("read", move |dir| read_blocking(dir, &path))
    }

    pub fn list(&self, path: String) -> FutureResult<Vec<DirEntry>> {
        self.run_blocking("list", move |dir| list_blocking(dir, &path))
    }

    pub fn write(&self, path: String, bytes: Vec<u8>) -> FutureResult<()> {
        if !self.writable {
            return ready_err(anyhow!("working tree is read-only; write to `{path}` denied"));
        }
        self.run_blocking("write", move |dir| write_blocking(dir, &path, &bytes))
    }
}

// A ready future already resolved to `err`.
fn ready_err<R: Send + 'static>(err: anyhow::Error) -> FutureResult<R> {
    async move { Err(err) }.boxed()
}

// Run `f` against a lent tree, or fail with a grant error.
pub fn with_tree<R: Send + 'static>(
    tree: Option<&WorkingTree>, tool: &'static str, f: impl FnOnce(&WorkingTree) -> FutureResult<R>,
) -> FutureResult<R> {
    tree.map_or_else(
        || ready_err(anyhow!("tool `{tool}` missing grants.working-tree")),
        |tree| f(tree),
    )
}

// Resolve a `grants.working-tree` into a [`WorkingTree`].
pub fn resolve(
    table: &ResourceTable, registry: &MountRegistry, borrow: Option<&Resource<Descriptor>>,
) -> anyhow::Result<Option<WorkingTree>> {
    let Some(resource) = borrow else {
        return Ok(None);
    };

    let descriptor = table.get(resource).context("resolving the lent working-tree descriptor")?;

    let Descriptor::Dir(dir) = descriptor else {
        bail!("grants.working-tree must be a directory descriptor, not a file");
    };

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

        let is_directory = entry.file_type().is_ok_and(|file_type| file_type.is_dir());
        entries.push(DirEntry {
            name: entry.file_name().to_string_lossy().into_owned(),
            is_directory,
        });
    }
    Ok(entries)
}

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
    use omnia::{MountRegistry, ResolvedPreopen};
    use wasmtime_wasi::filesystem::{Descriptor, Dir, File, OpenMode};
    use wasmtime_wasi::{DirPerms, FilePerms, ResourceTable};

    use super::resolve;

    /// A fresh, empty temp directory unique to this process and `label`.
    fn temp_root(label: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("omnia-wt-{label}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("creating temp working-tree root");
        dir
    }

    /// A single-mount registry named `.` over `path`, mirroring how a `[[mount]]`
    /// resolves (read-only unless `writable`).
    fn registry_for(path: &Path, writable: bool) -> MountRegistry {
        MountRegistry::open(vec![ResolvedPreopen::new(
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
    fn no_grant() {
        let table = ResourceTable::new();
        let registry = MountRegistry::default();
        let resolved = resolve(&table, &registry, None).expect("resolve");
        assert!(resolved.is_none(), "an absent grant resolves to None");
    }

    #[test]
    fn authorized_directory() {
        let root = temp_root("match");
        let registry = registry_for(&root, false);

        let mut table = ResourceTable::new();
        let resource = table.push(dir_descriptor(&root, false)).expect("pushing descriptor");

        let resolved = resolve(&table, &registry, Some(&resource))
            .expect("resolve succeeds for an authorized mount")
            .expect("an authorized mount resolves to a working tree");
        assert_eq!(
            resolved.local_path(),
            root.as_path(),
            "local_path is the registry mount's host path, never read from the descriptor"
        );
    }

    #[tokio::test]
    async fn read_tree() {
        let root = temp_root("read");
        std::fs::write(root.join("hello.txt"), b"hi").expect("seeding a file");
        let registry = registry_for(&root, false);
        let mut table = ResourceTable::new();
        let resource = table.push(dir_descriptor(&root, false)).expect("pushing descriptor");

        let tree = resolve(&table, &registry, Some(&resource))
            .expect("resolve")
            .expect("authorized mount");
        let bytes = tree.read("hello.txt".to_owned()).await.expect("reading within the mount");
        assert_eq!(bytes, b"hi", "read returns the file's bytes");
    }

    #[test]
    fn out_of_scope_directory() {
        let authorized = temp_root("scope-ok");
        let other = temp_root("scope-bad");
        let registry = registry_for(&authorized, false);

        let mut table = ResourceTable::new();
        // A directory that is *not* an authorized mount (a sibling tree).
        let resource = table.push(dir_descriptor(&other, false)).expect("pushing descriptor");

        let Err(err) = resolve(&table, &registry, Some(&resource)) else {
            panic!("an unauthorized tree must be rejected in the host");
        };
        assert!(
            format!("{err:#}").contains("out of scope"),
            "rejection names the out-of-scope cause: {err:#}"
        );
    }

    #[test]
    fn file_descriptor() {
        let root = temp_root("nondir");
        std::fs::write(root.join("not-a-dir.txt"), b"x").expect("seeding a file");
        let registry = registry_for(&root, false);

        let cap_dir = CapDir::open_ambient_dir(&root, ambient_authority()).expect("opening dir");
        let cap_file = cap_dir.open("not-a-dir.txt").expect("opening file");
        let descriptor =
            Descriptor::File(File::new(cap_file, FilePerms::READ, OpenMode::READ, false));
        let mut table = ResourceTable::new();
        let resource = table.push(descriptor).expect("pushing descriptor");

        let Err(err) = resolve(&table, &registry, Some(&resource)) else {
            panic!("a file descriptor must not resolve to a working tree");
        };
        assert!(
            format!("{err:#}").contains("must be a directory"),
            "rejection names the non-directory cause: {err:#}"
        );
    }

    #[tokio::test]
    async fn write_denied() {
        let root = temp_root("ro");
        let registry = registry_for(&root, false);
        let mut table = ResourceTable::new();
        let resource = table.push(dir_descriptor(&root, false)).expect("pushing descriptor");

        let tree = resolve(&table, &registry, Some(&resource))
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
