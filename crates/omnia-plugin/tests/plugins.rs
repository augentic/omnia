//! End-to-end tests for the `omnia:plugins/loader` host capability: a real
//! requester guest from `crates/test-programs` drives loads through omnia's
//! runtime, with `PathMounts` reading components staged in a scratch mount or
//! a `RegistryClient` resolving them from a wasm-pkg `local` backend. The
//! requester asserts internally (handles, digests, dispatch answers, and
//! every typed refusal); the host side stages artifacts and checks the exit.
//! Lifecycle scenarios the WASI surface cannot reach (deregistration) drive
//! [`LoadPlugin`] host-side over the same runtime.

#![cfg(not(target_arch = "wasm32"))]

use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context as _, Result};
use omnia::{
    Acquirer, DeploymentBuilder, ExitStatus, GuestArtifact, GuestEntry, LoadError, LoadPlugin as _,
    Location, Manifest, Mode, PathMounts, Plugins, RegistryClient, Runtime, StoreCtx, WasiPlugins,
    sha256_digest,
};

// Every guest program in `crates/test-programs/programs/plugins` must have a
// matching test here; a new program without one fails to compile.
test_utils::foreach_plugins!();

/// Assemble the requester deployment around `wasm`: the scratch dir mounts at
/// `.`, `omnia-test:link/ops` is the declared plugin seam, and `acquirer` is
/// the compiled-in acquisition policy.
async fn requester_runtime(
    wasm: &str, scratch: &test_utils::Scratch, acquirer: Acquirer,
) -> Result<Runtime<()>> {
    let manifest = Manifest::new()
        .plugins(["omnia-test:link/ops"])
        .guest(GuestEntry::new("requester", wasm))
        .mounts([scratch.mount(false)]);
    let deployment = DeploymentBuilder::new()
        .manifest(manifest)
        .mode(Mode::Command)
        .build::<StoreCtx<()>>()
        .await
        .context("building deployment")?;
    Runtime::new(
        deployment,
        |deployment| {
            deployment.host::<WasiPlugins, ()>()?;
            Ok(())
        },
        move |runtime| Plugins::install(runtime, acquirer),
    )
    .await
    .context("assembling runtime")
}

/// Drive `wasm` as the `requester` command guest under `acquirer`.
async fn run_requester(
    wasm: &str, scratch: &test_utils::Scratch, acquirer: Acquirer,
) -> Result<ExitStatus> {
    let runtime = requester_runtime(wasm, scratch, acquirer).await?;
    runtime.run_command().await
}

/// A `.`-rooted `PathMounts` over the scratch dir filling the path slot.
fn path_acquirer(scratch: &test_utils::Scratch) -> Acquirer {
    let paths = PathMounts::new([(".", scratch.path())]).expect("opening the scratch location");
    Acquirer {
        path: Some(Arc::new(paths)),
        registry: None,
    }
}

#[derive(serde::Serialize)]
struct LocalBackendConfig {
    root: PathBuf,
}

/// Stage `wasm` as `package` in a wasm-pkg `local` backend rooted at `root`
/// and return an acquirer whose default registry `registry.test` serves it.
fn registry_acquirer(root: &Path, package: &str, wasm: &str) -> Acquirer {
    let (name, version) = package.split_once('@').expect("test packages pin versions");
    let (namespace, name) = name.split_once(':').expect("test packages are namespaced");
    let dir = root.join(namespace).join(name);
    std::fs::create_dir_all(&dir).expect("creating package directory");
    std::fs::copy(wasm, dir.join(format!("{version}.wasm"))).expect("staging package");

    let registry: wasm_pkg_client::Registry =
        "registry.test".parse().expect("test registry name parses");
    let mut config = wasm_pkg_client::Config::empty();
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
    Acquirer {
        registry: Some(Arc::new(RegistryClient::new("registry.test").with_config(config))),
        path: None,
    }
}

#[tokio::test]
async fn plugins_load_path() {
    let scratch = test_utils::scratch("plugins_load_path");
    std::fs::copy(test_utils::LINK_ECHOER, scratch.path().join("plugin.wasm"))
        .expect("staging the loadable echoer");

    let status = run_requester(test_utils::PLUGINS_LOAD_PATH, &scratch, path_acquirer(&scratch))
        .await
        .expect("deployment runs");
    assert_eq!(status, ExitStatus::SUCCESS, "the requester's assertions all held");
}

#[tokio::test]
async fn plugins_load_registry() {
    let scratch = test_utils::scratch("plugins_load_registry");
    let acquirer = registry_acquirer(scratch.path(), "test:echoer@1.0.0", test_utils::LINK_ECHOER);

    let status = run_requester(test_utils::PLUGINS_LOAD_REGISTRY, &scratch, acquirer)
        .await
        .expect("deployment runs");
    assert_eq!(status, ExitStatus::SUCCESS, "the requester's assertions all held");
}

// The guest-side copies exist because published crates cannot reference WIT
// outside their package root; the host copy stays canonical.
#[test]
fn wit_copies_stay_identical() {
    let canonical = include_str!("../wit/plugins.wit");
    assert_eq!(
        include_str!("../../omnia-guest/wit/plugins.wit"),
        canonical,
        "omnia-guest's plugins.wit copy drifted from crates/omnia-plugin/wit/plugins.wit"
    );
    assert_eq!(
        include_str!("../../test-programs/wit/deps/plugins/plugins.wit"),
        canonical,
        "test-programs' plugins.wit copy drifted from crates/omnia-plugin/wit/plugins.wit"
    );
}

// Compile-time proof that the macro's `locations:`/`cache:` grammar lowers
// into calls that typecheck against this crate's public seam: path entries
// fold into a `PathMounts`, the registry entry into a `RegistryClient`
// cached in the `cache:` backend, each filling its slot in the `Acquirer`
// installed by the `Wiring::extend` hook.
// Never run — the macro's snapshot suite pins the shape, this pins the types.
mod locations_grammar {
    use std::future::Future;

    use anyhow::Result;
    use omnia::futures::future::BoxFuture;
    use omnia::{Backend, ContentStore, NoOptions, NoStore, ReleaseRecord, ReleaseStore};

    // Clone without Copy: the generated hook clones the backend out of the
    // bundle, and a Copy type would trip `clippy::clone_on_copy` there.
    #[derive(Clone)]
    struct Cache;

    impl Backend for Cache {
        type ConnectOptions = NoOptions;

        fn connect_with(_options: NoOptions) -> impl Future<Output = Result<Self>> {
            std::future::ready(Ok(Self))
        }
    }

    impl ContentStore for Cache {
        fn content<'a>(&'a self, digest: &'a str) -> BoxFuture<'a, Result<Option<Vec<u8>>>> {
            ContentStore::content(&NoStore, digest)
        }

        fn put_content<'a>(
            &'a self, digest: &'a str, bytes: &'a [u8],
        ) -> BoxFuture<'a, Result<()>> {
            ContentStore::put_content(&NoStore, digest, bytes)
        }
    }

    impl ReleaseStore for Cache {
        fn release<'a>(
            &'a self, registry: &'a str, package: &'a str, version: &'a str,
        ) -> BoxFuture<'a, Result<Option<ReleaseRecord>>> {
            ReleaseStore::release(&NoStore, registry, package, version)
        }

        fn put_release<'a>(
            &'a self, registry: &'a str, package: &'a str, record: &'a ReleaseRecord,
        ) -> BoxFuture<'a, Result<()>> {
            ReleaseStore::put_release(&NoStore, registry, package, record)
        }
    }

    omnia::runtime!({
        plugins: {
            interfaces: ["omnia-test:link/ops"],
            locations: [
                { name: "adapters", path: "adapters" },
                { registry: "ghcr.io" },
            ],
            cache: Cache,
        },
        guests: [
            { id: "engine", source: "engine.wasm" },
        ],
    });
}

#[test]
fn locations_grammar_expands() {
    // Touch the generated entry points so the compile-only module above is
    // reachable for dead-code analysis.
    let _ = (locations_grammar::main, locations_grammar::run);
}

#[tokio::test]
async fn plugins_load_refused() {
    let scratch = test_utils::scratch("plugins_load_refused");
    std::fs::copy(test_utils::LINK_ECHOER, scratch.path().join("plugin.wasm"))
        .expect("staging the loadable echoer");
    // Exports no `omnia-test:link/ops` instance — the seam-missing target.
    std::fs::copy(test_utils::LINK_FULL, scratch.path().join("noseam.wasm"))
        .expect("staging the seamless component");
    // Leading ELF magic is exactly what the loader sniffs; the tail is junk,
    // proving refusal happens before any wasmtime parsing.
    std::fs::write(scratch.path().join("native.bin"), [0x7f, b'E', b'L', b'F', 0, 0, 0, 0])
        .expect("staging native bytes");

    let status = run_requester(test_utils::PLUGINS_LOAD_REFUSED, &scratch, path_acquirer(&scratch))
        .await
        .expect("deployment runs");
    assert_eq!(status, ExitStatus::SUCCESS, "the requester's assertions all held");
}

/// A wasm custom section (id 0) named `omnia-test` wrapping `payload`:
/// appending one changes a component's bytes — and digest — without changing
/// its behavior. Single-byte LEB128 sizes, so name plus payload stay short.
fn custom_section(payload: &[u8]) -> Vec<u8> {
    let name = b"omnia-test";
    let mut body = vec![u8::try_from(name.len()).expect("short name")];
    body.extend_from_slice(name);
    body.extend_from_slice(payload);
    let mut section = vec![0x00, u8::try_from(body.len()).expect("short section")];
    section.extend_from_slice(&body);
    section
}

// Host-side loads over the same runtime the guests drive: deregistration is
// host authority, so the WASI surface cannot reach this scenario. A
// deregistered package's digest record must not survive into the next load —
// the re-load binds the freshly staged bytes, and a stale pin refuses.
#[tokio::test]
async fn reload_after_deregister_binds_fresh_bytes() {
    let scratch = test_utils::scratch("plugins_reload");
    std::fs::copy(test_utils::LINK_ECHOER, scratch.path().join("plugin.wasm"))
        .expect("staging the loadable echoer");
    // The requester guest is manifest ballast: these loads are host-driven.
    let runtime =
        requester_runtime(test_utils::PLUGINS_LOAD_PATH, &scratch, path_acquirer(&scratch))
            .await
            .expect("assembling runtime");
    let location = || Location::Path("./plugin.wasm".to_owned());

    let first = runtime.load_plugin("test:echoer", location(), None).await.expect("first load");
    runtime.deregister(first.id()).expect("deregistering the loaded plugin");

    // Same component, one extra custom section: same behavior, new digest.
    let mut changed = std::fs::read(test_utils::LINK_ECHOER).expect("reading the echoer");
    changed.extend_from_slice(&custom_section(b"reload"));
    std::fs::write(scratch.path().join("plugin.wasm"), &changed).expect("re-staging");

    let stale = runtime
        .load_plugin("test:echoer", location(), Some(first.digest()))
        .await
        .expect_err("the old digest no longer matches the staged bytes");
    match &stale {
        LoadError::Refused(detail) => {
            assert!(detail.contains("does not match the pinned"), "{detail}");
        }
        other => panic!("expected a digest-mismatch refusal: {other:?}"),
    }

    let fresh = runtime.load_plugin("test:echoer", location(), None).await.expect("re-load");
    assert_ne!(fresh.digest(), first.digest(), "the re-load bound fresh bytes");
    assert_eq!(fresh.digest(), sha256_digest(&changed));
    runtime.shutdown();
}

// The digest record lives on the registry entry, so an embedder swapping the
// identity outside the load path (deregister + `Runtime::register`) leaves no
// stale attestation behind: a pinned re-load must refuse rather than answer
// with the old digest over the new bytes.
#[tokio::test]
async fn pinned_reload_refuses_after_external_reregistration() {
    let scratch = test_utils::scratch("plugins_reregister");
    std::fs::copy(test_utils::LINK_ECHOER, scratch.path().join("plugin.wasm"))
        .expect("staging the loadable echoer");
    let runtime =
        requester_runtime(test_utils::PLUGINS_LOAD_PATH, &scratch, path_acquirer(&scratch))
            .await
            .expect("assembling runtime");
    let location = || Location::Path("./plugin.wasm".to_owned());

    let first = runtime.load_plugin("test:echoer", location(), None).await.expect("first load");
    runtime.deregister(first.id()).expect("deregistering the loaded plugin");

    // Same component, one extra custom section: same behavior, new digest —
    // registered by the embedder, not through the loader.
    let mut changed = std::fs::read(test_utils::LINK_ECHOER).expect("reading the echoer");
    changed.extend_from_slice(&custom_section(b"reregister"));
    runtime
        .register("test:echoer", GuestArtifact::wasm(changed))
        .await
        .expect("re-registering externally");

    let stale = runtime
        .load_plugin("test:echoer", location(), Some(first.digest()))
        .await
        .expect_err("a load never attests an externally registered guest");
    assert!(matches!(stale, LoadError::AlreadyActive(_)), "{stale:?}");
    runtime.shutdown();
}
