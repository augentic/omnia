//! Acquisition over wasm-pkg-client's `local` backend: fresh-release-preferred
//! resolution, the store as fallback and byte cache, poisoned entries,
//! endpoint overrides, path locations, and acquirer composition — all
//! offline.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use futures::FutureExt as _;
use futures::future::BoxFuture;
use omnia_plugin::{
    Acquire, AcquireError, AcquireExt as _, Location, PathAcquire, PluginStore, RegistryAcquire,
    ReleaseRecord, sha256_digest,
};
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

/// A cacheless acquirer whose default registry is a local backend at `root`.
fn registry_acquirer(root: &Path) -> RegistryAcquire {
    let mut config = Config::empty();
    add_local_registry(&mut config, DEFAULT_REGISTRY, root);
    RegistryAcquire::new(DEFAULT_REGISTRY).with_config(config)
}

type ReleaseKey = (String, String, String);

/// An in-memory [`PluginStore`] double: digest-keyed content plus
/// per-registry release records, with direct map access so tests can
/// inspect and poison entries without going through the trait.
#[derive(Clone, Default)]
struct MemStore {
    content: Arc<Mutex<HashMap<String, Vec<u8>>>>,
    releases: Arc<Mutex<HashMap<ReleaseKey, ReleaseRecord>>>,
}

impl MemStore {
    fn content_of(&self, digest: &str) -> Option<Vec<u8>> {
        self.content.lock().expect("content lock").get(digest).cloned()
    }

    fn poison(&self, digest: &str, bytes: &[u8]) {
        self.content.lock().expect("content lock").insert(digest.to_owned(), bytes.to_vec());
    }
}

impl PluginStore for MemStore {
    fn get_content<'a>(
        &'a self, digest: &'a str,
    ) -> BoxFuture<'a, anyhow::Result<Option<Vec<u8>>>> {
        let bytes = self.content_of(digest);
        async move { Ok(bytes) }.boxed()
    }

    fn put_content<'a>(
        &'a self, digest: &'a str, bytes: &'a [u8],
    ) -> BoxFuture<'a, anyhow::Result<()>> {
        self.content.lock().expect("content lock").insert(digest.to_owned(), bytes.to_vec());
        async move { Ok(()) }.boxed()
    }

    fn get_release<'a>(
        &'a self, registry: &'a str, package: &'a str, version: &'a str,
    ) -> BoxFuture<'a, anyhow::Result<Option<ReleaseRecord>>> {
        let key = (registry.to_owned(), package.to_owned(), version.to_owned());
        let record = self.releases.lock().expect("release lock").get(&key).cloned();
        async move { Ok(record) }.boxed()
    }

    fn put_release<'a>(
        &'a self, registry: &'a str, package: &'a str, record: &'a ReleaseRecord,
    ) -> BoxFuture<'a, anyhow::Result<()>> {
        let key = (registry.to_owned(), package.to_owned(), record.version.clone());
        self.releases.lock().expect("release lock").insert(key, record.clone());
        async move { Ok(()) }.boxed()
    }
}

async fn acquire(
    acquirer: &impl Acquire, package: &str, from: Location,
) -> Result<Vec<u8>, AcquireError> {
    acquirer.acquire(package, &from).await
}

/// The failure text of an [`AcquireError::Failed`], context chain included.
fn failure_text(error: &AcquireError) -> String {
    match error {
        AcquireError::Failed(error) => format!("{error:#}"),
        AcquireError::Unsupported(reason) => {
            panic!("expected a failure, got unsupported: {reason}")
        }
    }
}

#[tokio::test]
async fn registry_fetch_round_trips() {
    let registry = TempDir::new().expect("registry dir");
    stage(registry.path(), PACKAGE, b"component bytes");
    let acquirer = registry_acquirer(registry.path()).cached(MemStore::default());

    let bytes = acquire(&acquirer, PACKAGE, Location::Registry(None)).await.expect("acquires");
    assert_eq!(bytes, b"component bytes");
}

#[tokio::test]
async fn store_miss_then_populates() {
    let registry = TempDir::new().expect("registry dir");
    stage(registry.path(), PACKAGE, b"component bytes");
    let store = MemStore::default();
    let acquirer = registry_acquirer(registry.path()).cached(store.clone());

    let digest = sha256_digest(b"component bytes");
    assert!(store.content_of(&digest).is_none(), "the store starts empty");
    acquire(&acquirer, PACKAGE, Location::Registry(None)).await.expect("acquires");
    assert!(store.content_of(&digest).is_some(), "the store gains the digest-keyed entry");
}

#[tokio::test]
async fn fresh_release_preferred_over_warm_store() {
    let registry = TempDir::new().expect("registry dir");
    stage(registry.path(), PACKAGE, b"first bytes");
    let acquirer = registry_acquirer(registry.path()).cached(MemStore::default());
    acquire(&acquirer, PACKAGE, Location::Registry(None)).await.expect("warms the store");

    // The registry re-publishes the same version with different content. A
    // release-record cache would keep serving the stored bytes; the fresh
    // resolution must win.
    stage(registry.path(), PACKAGE, b"second bytes");
    let bytes = acquire(&acquirer, PACKAGE, Location::Registry(None)).await.expect("re-acquires");
    assert_eq!(bytes, b"second bytes", "the reachable registry is the authority");
}

#[tokio::test]
async fn network_failure_falls_back_to_stored_record() {
    let registry = TempDir::new().expect("registry dir");
    stage(registry.path(), PACKAGE, b"component bytes");
    let store = MemStore::default();

    // Warm the store under the unroutable registry *name*, served by a
    // local backend mapping.
    let mut config = Config::empty();
    add_local_registry(&mut config, UNROUTABLE_REGISTRY, registry.path());
    let warm = RegistryAcquire::new(UNROUTABLE_REGISTRY).with_config(config).cached(store.clone());
    acquire(&warm, PACKAGE, Location::Registry(None)).await.expect("warms the store");

    // Same registry name and store, no backend mapping: resolution now dials
    // the closed port and fails as a network error, so the stored record and
    // content serve the load.
    let offline = RegistryAcquire::new(UNROUTABLE_REGISTRY).cached(store);
    let bytes = acquire(&offline, PACKAGE, Location::Registry(None)).await.expect("falls back");
    assert_eq!(bytes, b"component bytes");
}

#[tokio::test]
async fn network_failure_without_record_refuses() {
    let acquirer = RegistryAcquire::new(UNROUTABLE_REGISTRY).cached(MemStore::default());

    let error = acquire(&acquirer, PACKAGE, Location::Registry(None))
        .await
        .expect_err("nothing stored to fall back to");
    assert!(failure_text(&error).contains("resolving"), "resolution failure: {error:?}");
}

#[tokio::test]
async fn poisoned_store_entry_discarded_and_refetched() {
    let registry = TempDir::new().expect("registry dir");
    stage(registry.path(), PACKAGE, b"honest bytes");
    let store = MemStore::default();
    let acquirer = registry_acquirer(registry.path()).cached(store.clone());
    acquire(&acquirer, PACKAGE, Location::Registry(None)).await.expect("warms the store");

    let digest = sha256_digest(b"honest bytes");
    store.poison(&digest, b"poison");

    let bytes = acquire(&acquirer, PACKAGE, Location::Registry(None))
        .await
        .expect("a poisoned entry refetches");
    assert_eq!(bytes, b"honest bytes");
    let healed = store.content_of(&digest).expect("reading the store entry");
    assert_eq!(healed, b"honest bytes", "the refetch overwrites the poisoned entry");
}

#[tokio::test]
async fn release_records_scoped_per_registry() {
    let default_root = TempDir::new().expect("default registry dir");
    stage(default_root.path(), PACKAGE, b"default registry bytes");
    let override_root = TempDir::new().expect("override registry dir");
    stage(override_root.path(), PACKAGE, b"override registry bytes");

    let mut config = Config::empty();
    add_local_registry(&mut config, DEFAULT_REGISTRY, default_root.path());
    add_local_registry(&mut config, "override.test", override_root.path());
    let acquirer =
        RegistryAcquire::new(DEFAULT_REGISTRY).with_config(config).cached(MemStore::default());

    let default_bytes =
        acquire(&acquirer, PACKAGE, Location::Registry(None)).await.expect("default acquires");
    assert_eq!(default_bytes, b"default registry bytes");
    // Same package and version, same store: release records are scoped per
    // registry, so the override never answers from the default's record.
    let override_bytes =
        acquire(&acquirer, PACKAGE, Location::Registry(Some("override.test".into())))
            .await
            .expect("override acquires");
    assert_eq!(override_bytes, b"override registry bytes");
}

#[tokio::test]
async fn cacheless_acquirer_fetches_fresh() {
    let registry = TempDir::new().expect("registry dir");
    stage(registry.path(), PACKAGE, b"first bytes");
    let acquirer = registry_acquirer(registry.path());

    let first = acquire(&acquirer, PACKAGE, Location::Registry(None)).await.expect("acquires");
    assert_eq!(first, b"first bytes");
    stage(registry.path(), PACKAGE, b"second bytes");
    let second = acquire(&acquirer, PACKAGE, Location::Registry(None)).await.expect("re-acquires");
    assert_eq!(second, b"second bytes", "nothing cached anywhere");
}

#[tokio::test]
async fn unversioned_and_missing_packages_refuse_typed() {
    let registry = TempDir::new().expect("registry dir");
    stage(registry.path(), PACKAGE, b"component bytes");
    let acquirer = registry_acquirer(registry.path());

    let unversioned = acquire(&acquirer, "test:adapter", Location::Registry(None))
        .await
        .expect_err("exact version is mandatory");
    assert!(failure_text(&unversioned).contains("exact version"), "refusal: {unversioned:?}");

    let missing = acquire(&acquirer, "test:absent@1.0.0", Location::Registry(None))
        .await
        .expect_err("an absent package fails");
    assert!(matches!(missing, AcquireError::Failed(_)), "typed failure: {missing:?}");
}

#[tokio::test]
async fn path_location_unsupported() {
    let registry = TempDir::new().expect("registry dir");
    let acquirer = registry_acquirer(registry.path());

    let error = acquire(&acquirer, PACKAGE, Location::Path("adapters/x.wasm".into()))
        .await
        .expect_err("paths are not served");
    assert!(matches!(error, AcquireError::Unsupported(_)), "typed refusal: {error:?}");
}

#[tokio::test]
async fn path_acquire_serves_its_own_locations() {
    let root = TempDir::new().expect("location dir");
    std::fs::write(root.path().join("plugin.wasm"), b"located bytes").expect("staging component");
    let acquirer = PathAcquire::new([(".", root.path())]).expect("locations open at construction");

    let prefixed = acquire(&acquirer, PACKAGE, Location::Path("./plugin.wasm".into()))
        .await
        .expect("prefixed path reads");
    assert_eq!(prefixed, b"located bytes");
    let bare = acquire(&acquirer, PACKAGE, Location::Path("plugin.wasm".into()))
        .await
        .expect("bare path falls back to the `.` entry");
    assert_eq!(bare, b"located bytes");

    let escape = acquire(&acquirer, PACKAGE, Location::Path("./../secret.wasm".into()))
        .await
        .expect_err("escapes refused");
    assert!(matches!(escape, AcquireError::Failed(_)));
    let registry = acquire(&acquirer, PACKAGE, Location::Registry(None))
        .await
        .expect_err("registry locations are not served");
    assert!(matches!(registry, AcquireError::Unsupported(_)));
}

#[tokio::test]
async fn path_acquire_opens_fail_fast() {
    let error = PathAcquire::new([("adapters", "/no/such/directory")])
        .expect_err("a missing location refuses at construction");
    assert!(format!("{error:#}").contains("adapters"), "the refusal names the location: {error}");
}

#[tokio::test]
async fn or_falls_through_on_unsupported() {
    let location_root = TempDir::new().expect("location dir");
    std::fs::write(location_root.path().join("plugin.wasm"), b"located bytes")
        .expect("staging located component");
    let registry = TempDir::new().expect("registry dir");
    stage(registry.path(), PACKAGE, b"registry bytes");
    let composed = PathAcquire::new([(".", location_root.path())])
        .expect("locations open")
        .or(registry_acquirer(registry.path()));

    let located = acquire(&composed, PACKAGE, Location::Path("plugin.wasm".into()))
        .await
        .expect("paths serve first");
    assert_eq!(located, b"located bytes");
    let fetched = acquire(&composed, PACKAGE, Location::Registry(None))
        .await
        .expect("the registry serves the fall-through");
    assert_eq!(fetched, b"registry bytes");
}

#[tokio::test]
async fn or_propagates_failures() {
    let empty = TempDir::new().expect("empty registry dir");
    let stocked = TempDir::new().expect("stocked registry dir");
    stage(stocked.path(), PACKAGE, b"reachable bytes");
    let composed = registry_acquirer(empty.path()).or(registry_acquirer(stocked.path()));

    let error = acquire(&composed, PACKAGE, Location::Registry(None))
        .await
        .expect_err("a failure never falls through");
    assert!(matches!(error, AcquireError::Failed(_)), "second never consulted: {error:?}");
}

#[tokio::test]
async fn or_reports_both_refusals() {
    let registry = TempDir::new().expect("registry dir");
    let composed = registry_acquirer(registry.path()).or(registry_acquirer(registry.path()));

    let error = acquire(&composed, PACKAGE, Location::Path("x.wasm".into()))
        .await
        .expect_err("neither serves paths");
    let AcquireError::Unsupported(reason) = error else {
        panic!("expected a combined unsupported refusal");
    };
    assert_eq!(reason.matches("registry locations only").count(), 2, "both refusals: {reason}");
}
