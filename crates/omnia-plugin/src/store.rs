//! The package store behind [`RegistryClient`](crate::RegistryClient): every
//! package the runtime acquires passes through it.
//!
//! The store is the authority for a release on this machine: a reference it
//! holds is served from it with no network, and one it lacks is fetched from
//! the registry and written once. A stored release is final until removed —
//! the store never refreshes it — whoever wrote it: the fetcher, an
//! operator's `cp`, or `wkg get <reference> -o <dir>/`.

use std::fs::File;
use std::io::{self, Write as _};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use anyhow::{Context as _, Result};
use futures::FutureExt as _;
use futures::future::BoxFuture;
pub use omnia_backend::plugins::{PackageStore, Reference};

/// A store over one flat directory: `<root>/<namespace>_<name>@<version>.wasm`.
///
/// The layout is the contract. The fetcher writes it, `wkg get <reference> -o
/// <root>/` writes it, an operator's `cp` writes it, `rm` forgets a release,
/// and `ls` lists them. `get` reads the file the reference names and nothing
/// else; a missing root is an empty store, and `put` creates it. An entry is
/// complete or absent: `put` writes beside the target, syncs the bytes to
/// disk, and links them into place, so neither a reader nor a crash sees a
/// partial release, and a file already there is never replaced.
#[derive(Clone, Debug)]
pub struct FsStore {
    root: PathBuf,
}

impl FsStore {
    /// A store over `root`, which need not exist yet.
    #[must_use]
    pub fn open(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// The directory the store files releases in.
    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    fn path(&self, reference: &Reference) -> PathBuf {
        self.root.join(reference.file_name())
    }

    fn read(&self, reference: &Reference) -> Result<Option<Vec<u8>>> {
        let path = self.path(reference);
        match std::fs::read(&path) {
            Ok(bytes) => Ok(Some(bytes)),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(error) => {
                Err(error).with_context(|| format!("reading the stored `{}`", path.display()))
            }
        }
    }

    // A temp file beside the target, linked into place: `EEXIST` on the link
    // is the no-replace rule holding, not a fault. The temp name is unique to
    // this call, never shared by a concurrent `put` of the same release, and
    // opened `create_new` so a stale name is a loud error rather than a
    // truncation of whatever stands there.
    fn write(&self, reference: &Reference, bytes: &[u8]) -> Result<()> {
        static SEQUENCE: AtomicU64 = AtomicU64::new(0);

        let target = self.path(reference);
        std::fs::create_dir_all(&self.root)
            .with_context(|| format!("creating the store `{}`", self.root.display()))?;
        let temp = self.root.join(format!(
            ".{}.{}.{}.tmp",
            reference.file_name(),
            std::process::id(),
            SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        let filed = Self::file(&temp, &target, bytes);
        let _ = std::fs::remove_file(&temp);
        filed
    }

    // The bytes are durable before the link is: a release the store holds is
    // final, so a crash must leave the target absent, never short.
    fn file(temp: &Path, target: &Path, bytes: &[u8]) -> Result<()> {
        let mut file = File::options()
            .write(true)
            .create_new(true)
            .open(temp)
            .with_context(|| format!("creating `{}`", temp.display()))?;
        file.write_all(bytes).with_context(|| format!("writing `{}`", temp.display()))?;
        file.sync_all().with_context(|| format!("syncing `{}`", temp.display()))?;
        drop(file);
        match std::fs::hard_link(temp, target) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => Ok(()),
            Err(error) => Err(error).with_context(|| format!("filing `{}`", target.display())),
        }
    }
}

impl PackageStore for FsStore {
    fn get<'a>(&'a self, reference: &'a Reference) -> BoxFuture<'a, Result<Option<Vec<u8>>>> {
        async move { self.read(reference) }.boxed()
    }

    fn put<'a>(&'a self, reference: &'a Reference, bytes: &'a [u8]) -> BoxFuture<'a, Result<()>> {
        async move { self.write(reference, bytes) }.boxed()
    }

    fn describe(&self, reference: &Reference) -> String {
        format!("the store `{}` holds no `{}`", self.root.display(), reference.file_name())
    }
}

/// The fetch-through store, the default for
/// [`RegistryClient`](crate::RegistryClient): `get` misses and `put` drops.
#[derive(Clone, Copy, Debug, Default)]
pub struct NoStore;

impl PackageStore for NoStore {
    fn get<'a>(&'a self, _reference: &'a Reference) -> BoxFuture<'a, Result<Option<Vec<u8>>>> {
        async { Ok(None) }.boxed()
    }

    fn put<'a>(&'a self, _reference: &'a Reference, _bytes: &'a [u8]) -> BoxFuture<'a, Result<()>> {
        async { Ok(()) }.boxed()
    }

    fn describe(&self, _reference: &Reference) -> String {
        "no store is attached".to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reference(spelled: &str) -> Reference {
        spelled.parse().expect("an exact reference")
    }

    #[test]
    fn reference_spellings() {
        let tool = reference("acme:tool@1.2.3");
        assert_eq!(tool.package().to_string(), "acme:tool");
        assert_eq!(tool.version().to_string(), "1.2.3");
        assert_eq!(tool.file_name(), "acme_tool@1.2.3.wasm");
        let dev = reference("emery:type-script@0.14.0-dev");
        assert_eq!(dev.file_name(), "emery_type-script@0.14.0-dev.wasm");
        for malformed in ["acme:tool", "acme:tool@latest", "tool@1.2.3", "acme_tool@1.2.3"] {
            malformed.parse::<Reference>().expect_err("refused");
        }
    }

    // Only the file the reference names is read: an old `<name>.wasm`, the
    // colon spelling, and a `wkg get` temp file never become a release.
    #[tokio::test]
    async fn fs_store_reads_one_spelling() {
        let dir = tempfile::tempdir().expect("scratch dir");
        let store = FsStore::open(dir.path());
        let tool = reference("acme:tool@1.2.3");
        for off in ["tool.wasm", "acme:tool@1.2.3.wasm", ".wkg-get-tool", "acme_tool@1.2.3"] {
            std::fs::write(dir.path().join(off), b"not a release").expect("writing");
        }
        assert_eq!(store.get(&tool).await.expect("readable"), None);
        assert_eq!(
            store.describe(&tool),
            format!("the store `{}` holds no `acme_tool@1.2.3.wasm`", dir.path().display())
        );

        std::fs::write(dir.path().join("acme_tool@1.2.3.wasm"), b"release").expect("writing");
        assert_eq!(store.get(&tool).await.expect("readable"), Some(b"release".to_vec()));
    }

    // a missing root is an empty store, and the first `put` creates it
    #[tokio::test]
    async fn fs_store_creates_root() {
        let dir = tempfile::tempdir().expect("scratch dir");
        let root = dir.path().join("adapters");
        let store = FsStore::open(&root);
        let tool = reference("acme:tool@1.2.3");
        assert_eq!(store.get(&tool).await.expect("an absent root reads empty"), None);

        store.put(&tool, b"release").await.expect("put creates the root");
        assert_eq!(std::fs::read(root.join("acme_tool@1.2.3.wasm")).expect("filed"), b"release");
        assert_eq!(
            std::fs::read_dir(&root).expect("listing").count(),
            1,
            "no temp file is left beside the release"
        );
    }

    #[tokio::test]
    async fn fs_store_never_replaces() {
        let dir = tempfile::tempdir().expect("scratch dir");
        let store = FsStore::open(dir.path());
        let tool = reference("acme:tool@1.2.3");
        store.put(&tool, b"first").await.expect("first put");
        store.put(&tool, b"second").await.expect("a second put is not a fault");
        assert_eq!(store.get(&tool).await.expect("readable"), Some(b"first".to_vec()));
    }

    // racing puts of one release each file their own temp, so the one that
    // lands is whole and the rest are the no-replace rule holding
    #[test]
    fn fs_store_concurrent_puts() {
        let dir = tempfile::tempdir().expect("scratch dir");
        let store = FsStore::open(dir.path());
        let tool = reference("acme:tool@1.2.3");
        let payloads: Vec<Vec<u8>> = (0..16u8).map(|byte| vec![byte; 1 << 16]).collect();
        std::thread::scope(|scope| {
            for payload in &payloads {
                scope.spawn(|| {
                    futures::executor::block_on(store.put(&tool, payload))
                        .expect("every put lands");
                });
            }
        });

        let stored =
            futures::executor::block_on(store.get(&tool)).expect("readable").expect("filed");
        assert!(payloads.contains(&stored), "the release is one put's whole bytes");
        assert_eq!(
            std::fs::read_dir(dir.path()).expect("listing").count(),
            1,
            "no temp file is left beside the release"
        );
    }
}
