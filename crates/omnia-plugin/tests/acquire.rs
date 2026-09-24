//! Acquisition over wasm-pkg-client's `local` backend: fresh-release-preferred
//! resolution, the store as fallback and byte cache, poisoned entries,
//! configuration routing, and unrouted packages — all offline.

#![cfg(not(target_arch = "wasm32"))]

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use futures::FutureExt as _;
use futures::future::BoxFuture;
use omnia::Digest;
use omnia_plugin::{ContentStore, LoadError, RegistryClient, RegistrySource as _, ReleaseStore};
use tempfile::TempDir;
use wasm_pkg_client::{Config, Registry};

const PACKAGE: &str = "test:adapter@1.0.0";
const DEFAULT_REGISTRY: &str = "registry.test";
// A closed local port: connection refused immediately, no network reached.
const UNROUTABLE_REGISTRY: &str = "127.0.0.1:1";

#[derive(serde::Serialize)]
struct LocalBackendConfig {
    root: PathBuf,
}

/// Stage `bytes` as `package` in a local-backend registry rooted at `root`.
fn stage(root: &Path, package: &str, bytes: &[u8]) {
    let (name, version) = package.split_once('@').expect("test packages pin versions");
    let (namespace, name) = name.split_once(':').expect("test packages are namespaced");
    let dir = root.join(namespace).join(name);
    std::fs::create_dir_all(&dir).expect("creating package directory");
    std::fs::write(dir.join(format!("{version}.wasm")), bytes).expect("staging package");
}

/// Register a `local`-backend registry named `name` in `config`.
fn add_local_registry(config: &mut Config, name: &str, root: &Path) {
    let registry: Registry = name.parse().expect("test registry name parses");
    let backend = config.get_or_insert_registry_config_mut(&registry);
    backend.set_default_backend(Some("local".into()));
    backend
        .set_backend_config(
            "local",
            LocalBackendConfig {
                root: root.to_path_buf(),
            },
        )
        .expect("local backend config serializes");
}

/// An empty configuration whose default registry is `name`.
fn defaulting_to(name: &str) -> Config {
    let mut config = Config::empty();
    config.set_default_registry(Some(name.parse().expect("test registry name parses")));
    config
}

/// A cacheless acquirer whose default registry is a local backend at `root`.
fn registry_acquirer(root: &Path) -> RegistryClient {
    let mut config = defaulting_to(DEFAULT_REGISTRY);
    add_local_registry(&mut config, DEFAULT_REGISTRY, root);
    RegistryClient::new(config)
}

/// The store key of `bytes`: the `sha256:<hex>` the registry reports.
fn key(bytes: &[u8]) -> String {
    Digest::of(bytes).to_string()
}

type ReleaseKey = (String, String, String);

/// An in-memory [`ContentStore`] + [`ReleaseStore`] double: digest-keyed
/// content plus per-registry release records, with direct map access so
/// tests can inspect and poison entries without going through the traits.
#[derive(Clone, Default)]
struct MemStore {
    content: Arc<Mutex<HashMap<String, Vec<u8>>>>,
    releases: Arc<Mutex<HashMap<ReleaseKey, String>>>,
}

impl MemStore {
    fn content_of(&self, digest: &str) -> Option<Vec<u8>> {
        self.content.lock().expect("content lock").get(digest).cloned()
    }

    fn poison(&self, digest: &str, bytes: &[u8]) {
        self.content.lock().expect("content lock").insert(digest.to_owned(), bytes.to_vec());
    }
}

impl ContentStore for MemStore {
    fn content<'a>(&'a self, digest: &'a str) -> BoxFuture<'a, anyhow::Result<Option<Vec<u8>>>> {
        let bytes = self.content_of(digest);
        async move { Ok(bytes) }.boxed()
    }

    fn put_content<'a>(
        &'a self, digest: &'a str, bytes: &'a [u8],
    ) -> BoxFuture<'a, anyhow::Result<()>> {
        self.content.lock().expect("content lock").insert(digest.to_owned(), bytes.to_vec());
        async move { Ok(()) }.boxed()
    }
}

impl ReleaseStore for MemStore {
    fn release<'a>(
        &'a self, registry: &'a str, package: &'a str, version: &'a str,
    ) -> BoxFuture<'a, anyhow::Result<Option<String>>> {
        let key = (registry.to_owned(), package.to_owned(), version.to_owned());
        let digest = self.releases.lock().expect("release lock").get(&key).cloned();
        async move { Ok(digest) }.boxed()
    }

    fn put_release<'a>(
        &'a self, registry: &'a str, package: &'a str, version: &'a str, digest: &'a str,
    ) -> BoxFuture<'a, anyhow::Result<()>> {
        let key = (registry.to_owned(), package.to_owned(), version.to_owned());
        self.releases.lock().expect("release lock").insert(key, digest.to_owned());
        async move { Ok(()) }.boxed()
    }
}

#[tokio::test]
async fn registry_fetch() {
    let registry = TempDir::new().expect("registry dir");
    stage(registry.path(), PACKAGE, b"component bytes");
    let acquirer = registry_acquirer(registry.path()).cached(MemStore::default());

    let bytes = acquirer.acquire(PACKAGE).await.expect("acquires");
    assert_eq!(bytes, b"component bytes");
}

#[tokio::test]
async fn store_miss() {
    let registry = TempDir::new().expect("registry dir");
    stage(registry.path(), PACKAGE, b"component bytes");
    let store = MemStore::default();
    let acquirer = registry_acquirer(registry.path()).cached(store.clone());

    let digest = key(b"component bytes");
    assert!(store.content_of(&digest).is_none(), "the store starts empty");
    acquirer.acquire(PACKAGE).await.expect("acquires");
    assert!(store.content_of(&digest).is_some(), "the store gains the digest-keyed entry");
}

#[tokio::test]
async fn fresh_over_warm() {
    let registry = TempDir::new().expect("registry dir");
    stage(registry.path(), PACKAGE, b"first bytes");
    let acquirer = registry_acquirer(registry.path()).cached(MemStore::default());
    acquirer.acquire(PACKAGE).await.expect("warms the store");

    // The registry re-publishes the same version with different content. A
    // release-record cache would keep serving the stored bytes; the fresh
    // resolution must win.
    stage(registry.path(), PACKAGE, b"second bytes");
    let bytes = acquirer.acquire(PACKAGE).await.expect("re-acquires");
    assert_eq!(bytes, b"second bytes", "the reachable registry is the authority");
}

#[tokio::test]
async fn network_failure_fallback() {
    let registry = TempDir::new().expect("registry dir");
    stage(registry.path(), PACKAGE, b"component bytes");
    let store = MemStore::default();

    // Warm the store under the unroutable registry *name*, served by a
    // local backend mapping.
    let mut config = defaulting_to(UNROUTABLE_REGISTRY);
    add_local_registry(&mut config, UNROUTABLE_REGISTRY, registry.path());
    let warm = RegistryClient::new(config).cached(store.clone());
    warm.acquire(PACKAGE).await.expect("warms the store");

    // Same registry name and store, no backend mapping: resolution now dials
    // the closed port and fails as a network error, so the stored record and
    // content serve the load.
    let offline = RegistryClient::new(defaulting_to(UNROUTABLE_REGISTRY)).cached(store);
    let bytes = offline.acquire(PACKAGE).await.expect("falls back");
    assert_eq!(bytes, b"component bytes");
}

#[tokio::test]
async fn network_failure_no_record() {
    let acquirer =
        RegistryClient::new(defaulting_to(UNROUTABLE_REGISTRY)).cached(MemStore::default());

    let error = acquirer.acquire(PACKAGE).await.expect_err("nothing stored to fall back to");
    assert!(
        matches!(&error, LoadError::Unavailable(detail) if detail.contains("resolving")),
        "resolution failure: {error:?}"
    );
}

// A configuration that routes the package nowhere — no default, no mapping
// for its namespace — refuses before any registry is dialled, even though a
// registry that could serve it is configured.
#[tokio::test]
async fn unrouted_package() {
    let registry = TempDir::new().expect("registry dir");
    stage(registry.path(), PACKAGE, b"component bytes");
    let mut config = Config::empty();
    add_local_registry(&mut config, DEFAULT_REGISTRY, registry.path());
    let acquirer = RegistryClient::new(config);

    let error = acquirer.acquire(PACKAGE).await.expect_err("nothing routes the package");
    assert!(
        matches!(&error, LoadError::Refused(detail) if detail.contains("no registry routes") && detail.contains("`test` namespace")),
        "refusal names the namespace: {error:?}"
    );
}

#[tokio::test]
async fn poisoned_store() {
    let registry = TempDir::new().expect("registry dir");
    stage(registry.path(), PACKAGE, b"honest bytes");
    let store = MemStore::default();
    let acquirer = registry_acquirer(registry.path()).cached(store.clone());
    acquirer.acquire(PACKAGE).await.expect("warms the store");

    let digest = key(b"honest bytes");
    store.poison(&digest, b"poison");

    let bytes = acquirer.acquire(PACKAGE).await.expect("a poisoned entry refetches");
    assert_eq!(bytes, b"honest bytes");
    let healed = store.content_of(&digest).expect("reading the store entry");
    assert_eq!(healed, b"honest bytes", "the refetch overwrites the poisoned entry");
}

// Release records are scoped per registry: the same package and version
// routed to another registry never answers from the first one's record.
#[tokio::test]
async fn release_scoped() {
    let first_root = TempDir::new().expect("first registry dir");
    stage(first_root.path(), PACKAGE, b"first registry bytes");
    let second_root = TempDir::new().expect("second registry dir");
    stage(second_root.path(), PACKAGE, b"second registry bytes");
    let store = MemStore::default();

    let mut first = defaulting_to(DEFAULT_REGISTRY);
    add_local_registry(&mut first, DEFAULT_REGISTRY, first_root.path());
    let bytes = RegistryClient::new(first)
        .cached(store.clone())
        .acquire(PACKAGE)
        .await
        .expect("first acquires");
    assert_eq!(bytes, b"first registry bytes");

    let mut second = defaulting_to("second.test");
    add_local_registry(&mut second, "second.test", second_root.path());
    let bytes =
        RegistryClient::new(second).cached(store).acquire(PACKAGE).await.expect("second acquires");
    assert_eq!(bytes, b"second registry bytes");
}

// The configuration alone decides a package's registry: a package override
// first, then its namespace, then the default.
#[tokio::test]
async fn config_routing() {
    let default_root = TempDir::new().expect("default registry dir");
    stage(default_root.path(), PACKAGE, b"default registry bytes");
    stage(default_root.path(), "acme:ledger@2.1.0", b"default ledger bytes");
    let acme_root = TempDir::new().expect("acme registry dir");
    stage(acme_root.path(), "acme:ledger@2.1.0", b"acme registry bytes");
    stage(acme_root.path(), "acme:pinned@1.0.0", b"acme pinned bytes");
    let pinned_root = TempDir::new().expect("pinned registry dir");
    stage(pinned_root.path(), "acme:pinned@1.0.0", b"pinned registry bytes");

    let mut config = Config::from_toml(&format!(
        "default_registry = \"{DEFAULT_REGISTRY}\"\n\n\
         [namespace_registries]\nacme = \"acme.test\"\n\n\
         [package_registry_overrides]\n\"acme:pinned\" = \"pinned.test\"\n",
    ))
    .expect("routing config parses");
    add_local_registry(&mut config, DEFAULT_REGISTRY, default_root.path());
    add_local_registry(&mut config, "acme.test", acme_root.path());
    add_local_registry(&mut config, "pinned.test", pinned_root.path());
    let acquirer = RegistryClient::new(config);

    let unmapped = acquirer.acquire(PACKAGE).await.expect("default acquires");
    assert_eq!(unmapped, b"default registry bytes", "an unmapped namespace falls to the default");
    let namespaced = acquirer.acquire("acme:ledger@2.1.0").await.expect("acme acquires");
    assert_eq!(namespaced, b"acme registry bytes", "a mapped namespace routes past the default");
    let pinned = acquirer.acquire("acme:pinned@1.0.0").await.expect("pinned acquires");
    assert_eq!(pinned, b"pinned registry bytes", "a package override beats its namespace");
}

#[tokio::test]
async fn cacheless() {
    let registry = TempDir::new().expect("registry dir");
    stage(registry.path(), PACKAGE, b"first bytes");
    let acquirer = registry_acquirer(registry.path());

    let first = acquirer.acquire(PACKAGE).await.expect("acquires");
    assert_eq!(first, b"first bytes");
    stage(registry.path(), PACKAGE, b"second bytes");
    let second = acquirer.acquire(PACKAGE).await.expect("re-acquires");
    assert_eq!(second, b"second bytes", "nothing cached anywhere");
}

#[tokio::test]
async fn unversioned_and_missing() {
    let registry = TempDir::new().expect("registry dir");
    stage(registry.path(), PACKAGE, b"component bytes");
    let acquirer = registry_acquirer(registry.path());

    let unversioned =
        acquirer.acquire("test:adapter").await.expect_err("exact version is mandatory");
    assert!(
        matches!(&unversioned, LoadError::Refused(detail) if detail.contains("exact version")),
        "refusal: {unversioned:?}"
    );

    let absent = acquirer.acquire("test:absent@1.0.0").await.expect_err("an absent package fails");
    assert!(matches!(absent, LoadError::Refused(_)), "an authoritative miss refuses: {absent:?}");
}
