//! Acquisition over wasm-pkg-client's `local` backend, store first: a stored
//! release served with no registry, a fetched one verified and written once,
//! what the store never reads, configuration routing, the registry a load
//! names, and unrouted packages — all offline.

#![cfg(not(target_arch = "wasm32"))]

use std::path::{Path, PathBuf};

use omnia::wasmtime::Engine;
use omnia::{AcquireError, CompileOptions};
use omnia_plugin::{
    FsStore, NoStore, PackageStore, Reference, RegistryClient, RegistrySource as _,
};
use tempfile::TempDir;
use wasm_pkg_client::{Config, Registry};

const PACKAGE: &str = "test:adapter@1.0.0";
// the file the store keeps `PACKAGE` under
const STORED: &str = "test_adapter@1.0.0.wasm";
const DEFAULT_REGISTRY: &str = "registry.test";
// A closed local port: connection refused immediately, no network reached.
const UNROUTABLE_REGISTRY: &str = "127.0.0.1:1";

#[derive(serde::Serialize)]
struct LocalBackendConfig {
    root: PathBuf,
}

fn stage(root: &Path, package: &str, bytes: &[u8]) {
    let (name, version) = package.split_once('@').expect("test packages pin versions");
    let (namespace, name) = name.split_once(':').expect("test packages are namespaced");
    let dir = root.join(namespace).join(name);
    std::fs::create_dir_all(&dir).expect("creating package directory");
    std::fs::write(dir.join(format!("{version}.wasm")), bytes).expect("staging package");
}

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

fn defaulting_to(name: &str) -> Config {
    let mut config = Config::empty();
    config.set_default_registry(Some(name.parse().expect("test registry name parses")));
    config
}

// routes every package to a local backend at `root`
fn routed_to(root: &Path) -> Config {
    let mut config = defaulting_to(DEFAULT_REGISTRY);
    add_local_registry(&mut config, DEFAULT_REGISTRY, root);
    config
}

// storeless, defaulting to a local backend at `root`
fn registry_acquirer(root: &Path) -> RegistryClient {
    RegistryClient::new(routed_to(root), NoStore)
}

fn reference(package: &str) -> Reference {
    package.parse().expect("test references are exact")
}

// what `omnia compile` writes
fn precompiled(scratch: &Path) -> Vec<u8> {
    let target = scratch.join("echoer.bin");
    omnia::compile::compile(
        Path::new(test_programs::LINK_ECHOER),
        Some(target.clone()),
        None,
        &CompileOptions::default(),
    )
    .expect("compiling the echoer");
    let bytes = std::fs::read(target).expect("reading the compiled echoer");
    assert!(Engine::detect_precompiled(&bytes).is_some(), "what it wrote is pre-compiled");
    bytes
}

#[tokio::test]
async fn registry_fetch() {
    let registry = TempDir::new().expect("registry dir");
    stage(registry.path(), PACKAGE, b"component bytes");
    let acquirer = registry_acquirer(registry.path());

    let bytes = acquirer.acquire(PACKAGE, None).await.expect("acquires");
    assert_eq!(bytes, b"component bytes");
}

// A fetched release is written to the store once it has hashed, so the next
// acquisition is served from the store with no registry reached — the
// registry republishing the version changes nothing until the file goes.
#[tokio::test]
async fn fetched_then_stored() {
    let registry = TempDir::new().expect("registry dir");
    let store = TempDir::new().expect("store dir");
    stage(registry.path(), PACKAGE, b"first bytes");
    let acquirer = RegistryClient::new(routed_to(registry.path()), FsStore::open(store.path()));

    assert!(!store.path().join(STORED).exists(), "the store starts empty");
    let bytes = acquirer.acquire(PACKAGE, None).await.expect("fetches");
    assert_eq!(bytes, b"first bytes");
    let stored = std::fs::read(store.path().join(STORED)).expect("the release was written");
    assert_eq!(stored, b"first bytes", "under the reference's `_` spelling");

    stage(registry.path(), PACKAGE, b"second bytes");
    let bytes = acquirer.acquire(PACKAGE, None).await.expect("serves the store");
    assert_eq!(bytes, b"first bytes", "the stored release is final");

    std::fs::remove_file(store.path().join(STORED)).expect("removing the stored release");
    let bytes = acquirer.acquire(PACKAGE, None).await.expect("fetches again");
    assert_eq!(bytes, b"second bytes", "removing the file is the refresh");
}

// A file copied under a reference is served with no registry reached at
// all: nothing routes the package, and the registry would not answer.
#[tokio::test]
async fn stored_without_registry() {
    let store = TempDir::new().expect("store dir");
    std::fs::write(store.path().join(STORED), b"copied bytes").expect("copying into the store");
    let acquirer = RegistryClient::new(Config::empty(), FsStore::open(store.path()));

    let bytes = acquirer.acquire(PACKAGE, None).await.expect("the store answers");
    assert_eq!(bytes, b"copied bytes");
    let bytes = acquirer.acquire(PACKAGE, Some(UNROUTABLE_REGISTRY)).await.expect("still answers");
    assert_eq!(bytes, b"copied bytes", "a stored release is served whatever registry is named");
}

// Only the `namespace_name@version.wasm` spelling is a stored release: the
// `:` spelling, the bare name, and wkg's temporary files are never read.
#[tokio::test]
async fn off_reference_names_unread() {
    let store = TempDir::new().expect("store dir");
    for name in ["test:adapter@1.0.0.wasm", "adapter.wasm", ".wkg-get-test_adapter@1.0.0.wasm"] {
        std::fs::write(store.path().join(name), b"off-reference bytes").expect("writing a file");
    }
    let acquirer = RegistryClient::new(Config::empty(), FsStore::open(store.path()));

    let error = acquirer.acquire(PACKAGE, None).await.expect_err("nothing stored answers");
    assert!(
        matches!(&error, AcquireError::Refused(detail) if detail.contains("no registry routes") && detail.contains(STORED)),
        "the refusal names the file the store would read: {error:?}"
    );
}

// A configuration that routes the package nowhere — no default, no mapping
// for its namespace — refuses before any registry is dialled, naming the
// namespace and the store that holds no file for it.
#[tokio::test]
async fn unrouted_package() {
    let registry = TempDir::new().expect("registry dir");
    let store = TempDir::new().expect("store dir");
    stage(registry.path(), PACKAGE, b"component bytes");
    let mut config = Config::empty();
    add_local_registry(&mut config, DEFAULT_REGISTRY, registry.path());
    let acquirer = RegistryClient::new(config, FsStore::open(store.path()));

    let error = acquirer.acquire(PACKAGE, None).await.expect_err("nothing routes the package");
    let root = store.path().display().to_string();
    assert!(
        matches!(&error, AcquireError::Refused(detail) if detail.contains("no registry routes") && detail.contains("`test` namespace") && detail.contains(&root) && detail.contains(STORED)),
        "refusal names the namespace, the store, and the file: {error:?}"
    );
    assert!(!store.path().join(STORED).exists(), "nothing was written");

    let storeless = RegistryClient::new(Config::empty(), NoStore);
    let error = storeless.acquire(PACKAGE, None).await.expect_err("nothing routes the package");
    assert!(
        matches!(&error, AcquireError::Refused(detail) if detail.contains("no store is attached")),
        "refusal says no store could hold it: {error:?}"
    );
}

// The registry's digest is checked before anything is written, and a
// pre-compiled release is refused the same way — nothing of either lands in
// the store, so a later acquisition reaches the registry again.
#[tokio::test]
async fn precompiled_never_stored() {
    let registry = TempDir::new().expect("registry dir");
    let store = TempDir::new().expect("store dir");
    stage(registry.path(), PACKAGE, &precompiled(registry.path()));
    let acquirer = RegistryClient::new(routed_to(registry.path()), FsStore::open(store.path()));

    let error = acquirer.acquire(PACKAGE, None).await.expect_err("pre-compiled is refused");
    assert!(
        matches!(&error, AcquireError::Refused(detail) if detail.contains("pre-compiled")),
        "refusal: {error:?}"
    );
    assert!(!store.path().join(STORED).exists(), "nothing was written");

    stage(registry.path(), PACKAGE, b"raw bytes");
    let bytes = acquirer.acquire(PACKAGE, None).await.expect("the raw release fetches");
    assert_eq!(bytes, b"raw bytes");
}

// A store that cannot be read is a source that may recover, not a refusal.
#[tokio::test]
async fn unreadable_store() {
    let scratch = TempDir::new().expect("scratch dir");
    let root = scratch.path().join("not-a-directory");
    std::fs::write(&root, b"a file where the store root should be").expect("writing the file");
    let acquirer = RegistryClient::new(Config::empty(), FsStore::open(&root));

    let error = acquirer.acquire(PACKAGE, None).await.expect_err("the store cannot be read");
    assert!(matches!(error, AcquireError::Unavailable(_)), "unavailable: {error:?}");
}

// `put` never replaces: a release already stored stays as it is, and a
// missing root is created on the first write.
#[tokio::test]
async fn put_is_final() {
    let scratch = TempDir::new().expect("scratch dir");
    let root = scratch.path().join("store");
    let store = FsStore::open(&root);
    let reference = reference(PACKAGE);

    assert!(store.get(&reference).await.expect("a missing root reads empty").is_none());
    store.put(&reference, b"first").await.expect("the first write creates the root");
    store.put(&reference, b"second").await.expect("a second write is a no-op");
    let held = store.get(&reference).await.expect("the store reads").expect("the release");
    assert_eq!(held, b"first");
    assert_eq!(std::fs::read(root.join(STORED)).expect("the file"), b"first");
    let entries = std::fs::read_dir(&root).expect("listing the store").count();
    assert_eq!(entries, 1, "no temporary file is left behind");
}

#[tokio::test]
async fn network_failure() {
    let acquirer = RegistryClient::new(defaulting_to(UNROUTABLE_REGISTRY), NoStore);

    let error = acquirer.acquire(PACKAGE, None).await.expect_err("the registry is unreachable");
    assert!(
        matches!(&error, AcquireError::Unavailable(detail) if detail.contains("resolving")),
        "resolution failure: {error:?}"
    );
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
    let acquirer = RegistryClient::new(config, NoStore);

    let unmapped = acquirer.acquire(PACKAGE, None).await.expect("default acquires");
    assert_eq!(unmapped, b"default registry bytes", "an unmapped namespace falls to the default");
    let namespaced = acquirer.acquire("acme:ledger@2.1.0", None).await.expect("acme acquires");
    assert_eq!(namespaced, b"acme registry bytes", "a mapped namespace routes past the default");
    let pinned = acquirer.acquire("acme:pinned@1.0.0", None).await.expect("pinned acquires");
    assert_eq!(pinned, b"pinned registry bytes", "a package override beats its namespace");
}

// The registry a load names serves a namespace the configuration routes
// nowhere, and gives way to the configuration's routing — namespace or
// default — everywhere else: a load cannot re-route a package the deployment
// has placed.
#[tokio::test]
async fn endpoint_named_on_load() {
    let default_root = TempDir::new().expect("default registry dir");
    stage(default_root.path(), PACKAGE, b"default registry bytes");
    let acme_root = TempDir::new().expect("acme registry dir");
    stage(acme_root.path(), "acme:ledger@2.1.0", b"acme registry bytes");
    let named_root = TempDir::new().expect("named registry dir");
    stage(named_root.path(), PACKAGE, b"named registry bytes");
    stage(named_root.path(), "acme:ledger@2.1.0", b"named ledger bytes");
    stage(named_root.path(), "other:tool@1.0.0", b"named tool bytes");

    let mut unrouted = Config::from_toml("[namespace_registries]\nacme = \"acme.test\"\n")
        .expect("routing config parses");
    add_local_registry(&mut unrouted, "acme.test", acme_root.path());
    add_local_registry(&mut unrouted, "named.test", named_root.path());
    let acquirer = RegistryClient::new(unrouted, NoStore);
    let bytes = acquirer.acquire(PACKAGE, Some("named.test")).await.expect("named acquires");
    assert_eq!(bytes, b"named registry bytes", "an unrouted namespace takes the named registry");
    let routed = acquirer
        .acquire("acme:ledger@2.1.0", Some("named.test"))
        .await
        .expect_err("a routed namespace is not re-routed");
    assert!(
        matches!(&routed, AcquireError::Refused(detail) if detail.contains("routed to `acme.test`") && detail.contains("`named.test`")),
        "refusal names both registries: {routed:?}"
    );
    let same = acquirer
        .acquire("acme:ledger@2.1.0", Some("acme.test"))
        .await
        .expect("naming the routed registry agrees with it");
    assert_eq!(same, b"acme registry bytes");
    let malformed = acquirer
        .acquire("other:tool@1.0.0", Some("not a registry"))
        .await
        .expect_err("a malformed registry name is refused");
    assert!(
        matches!(&malformed, AcquireError::Refused(detail) if detail.contains("not a valid name")),
        "refusal: {malformed:?}"
    );

    let mut defaulted = defaulting_to(DEFAULT_REGISTRY);
    add_local_registry(&mut defaulted, DEFAULT_REGISTRY, default_root.path());
    add_local_registry(&mut defaulted, "named.test", named_root.path());
    let acquirer = RegistryClient::new(defaulted, NoStore);
    let error = acquirer
        .acquire(PACKAGE, Some("named.test"))
        .await
        .expect_err("a default registry routes every namespace");
    assert!(
        matches!(&error, AcquireError::Refused(detail) if detail.contains("routed to `registry.test`")),
        "refusal: {error:?}"
    );
}

#[tokio::test]
async fn unversioned_and_missing() {
    let registry = TempDir::new().expect("registry dir");
    stage(registry.path(), PACKAGE, b"component bytes");
    let acquirer = registry_acquirer(registry.path());

    let unversioned =
        acquirer.acquire("test:adapter", None).await.expect_err("exact version is mandatory");
    assert!(
        matches!(&unversioned, AcquireError::Refused(detail) if detail.contains("exact version")),
        "refusal: {unversioned:?}"
    );

    let absent =
        acquirer.acquire("test:absent@1.0.0", None).await.expect_err("an absent package fails");
    assert!(
        matches!(absent, AcquireError::Refused(_)),
        "an authoritative miss refuses: {absent:?}"
    );
}
