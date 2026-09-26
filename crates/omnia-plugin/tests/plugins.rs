//! End-to-end tests for the `omnia:plugins/loader` host capability: a real
//! requester guest from `crates/test-programs` drives loads through omnia's
//! runtime against a deployment whose grant bounds what may be loaded — the
//! `[[guest]]` list it declares (from a staged file, embedded bytes, or a
//! wasm-pkg `local` registry), each loaded at boot or at its first use as
//! its source decides, the read-only mounts a path may lie beneath, and the
//! registries its `registries` routes a package to. The requester
//! asserts internally (handles, digests, dispatch answers, and every typed
//! refusal); the host side declares the deployment and checks the exit and
//! the registry. Lifecycle scenarios the WASI surface cannot reach
//! (deregistration, embedder re-registration) drive [`PluginLoader`]
//! host-side over the same runtime.

#![cfg(not(target_arch = "wasm32"))]

use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result, bail};
use omnia::wasmtime::Engine;
use omnia::wasmtime::component::Val;
use omnia::{
    AcquireError, ChainCtx, CompileOptions, DeploymentBuilder, Digest, ExitStatus, GuestEntry,
    GuestId, LoadError, Location, Manifest, Mode, PluginLoader as _, RegistryClient,
    RegistryConfig, RegistrySource, Runtime, SourceSpec, StoreCtx,
};
use omnia_test::host::{Backends, Scratch, scratch};
use omnia_wasi_otel::WasiOtel;

// Every guest program in `crates/test-programs/programs/plugins` must have a
// matching test here; a new program without one fails to compile.
test_programs::foreach_plugins!();

const ECHOER_PACKAGE: &str = "test:echoer@1.0.0";

// the name the echoer package derives: its reference without the version
const ECHOER_NAME: &str = "test:echoer";

// a path, so the requester loads when the command drive first names it
fn requester(wasm: &str) -> Manifest {
    Manifest::new().guest(GuestEntry::new("requester", wasm).command())
}

fn declared(name: &str) -> Location {
    Location::Declared(name.to_owned())
}

// command mode; the loader host is assembly's, and the link seam is read
// off the components
async fn deployment(
    manifest: Manifest, args: &[&str],
) -> Result<omnia::Deployment<StoreCtx<Backends>>> {
    let mut deployment = DeploymentBuilder::new()
        .manifest(manifest)
        .mode(Mode::Command)
        .args(args.iter().map(|arg| (*arg).to_owned()).collect::<Vec<_>>())
        .build::<StoreCtx<Backends>>()
        .await
        .context("building deployment")?;
    deployment.host::<WasiOtel, Backends>()?;
    Ok(deployment)
}

// over the declared policy: package sources fetch through the manifest's `registries`
async fn boot(manifest: Manifest, args: &[&str]) -> Result<Runtime<Backends>> {
    deployment(manifest, args)
        .await?
        .assemble(Backends::defaults().await)
        .await
        .context("assembling runtime")
}

async fn run(manifest: Manifest, args: &[&str]) -> Result<ExitStatus> {
    let runtime = boot(manifest, args).await?;
    let status = runtime.run_command().await;
    runtime.shutdown();
    status
}

// host-to-guest `ping` through the `Dispatcher`, from a server root
async fn invoke_ping(runtime: &Runtime<Backends>, target: &str, message: &str) -> Result<String> {
    let results = runtime
        .dispatcher()
        .invoke(
            ChainCtx::server(),
            GuestId::from(target),
            Some("omnia-test:link/ops".to_owned()),
            "ping".to_owned(),
            vec![Val::String(target.to_owned()), Val::String(message.to_owned())],
        )
        .await
        .with_context(|| format!("dispatching ping to `{target}`"))?;
    match results.into_iter().next() {
        Some(Val::String(answer)) => Ok(answer),
        other => bail!("ping on `{target}` returned a non-string result: {other:?}"),
    }
}

// instantiates `guest` fresh and drives a world-level string export
async fn call_export(
    runtime: &Runtime<Backends>, guest: &str, func: &str, message: &str,
) -> Result<String> {
    let entry = runtime
        .registry()
        .get(&GuestId::from(guest))
        .with_context(|| format!("guest `{guest}` is not registered"))?;
    let mut store = runtime.build_store(runtime.store());
    let instance = runtime
        .instantiate(entry.instance_pre(), &mut store)
        .await
        .with_context(|| format!("instantiating `{guest}`"))?;
    let export = instance
        .get_func(&mut store, func)
        .with_context(|| format!("guest `{guest}` exports `{func}`"))?;
    let mut results = vec![Val::Bool(false)];
    export
        .call_async(&mut store, &[Val::String(message.to_owned())], &mut results)
        .await
        .map_err(anyhow::Error::from)
        .with_context(|| format!("calling `{guest}`'s `{func}`"))?;
    match results.into_iter().next() {
        Some(Val::String(answer)) => Ok(answer),
        other => bail!("`{guest}`'s `{func}` returned a non-string result: {other:?}"),
    }
}

// what the registry holds for a guest name
#[derive(Debug, PartialEq, Eq)]
enum Registered {
    Absent,
    /// Active, with the digest recorded at admission.
    Active(Digest),
}

fn recorded(runtime: &Runtime<Backends>, guest: &str) -> Registered {
    runtime
        .registry()
        .get(&GuestId::from(guest))
        .map_or(Registered::Absent, |entry| Registered::Active(entry.digest()))
}

fn digest_of(path: &str) -> Digest {
    Digest::of(&std::fs::read(path).unwrap_or_else(|error| panic!("reading {path}: {error}")))
}

fn stage(scratch: &Scratch, name: &str, wasm: &str) -> PathBuf {
    let target = scratch.path().join(name);
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent).expect("creating the staging directory");
    }
    std::fs::copy(wasm, &target).unwrap_or_else(|error| panic!("staging {name}: {error}"));
    target
}

// what `omnia compile` writes
fn precompile(scratch: &Scratch, name: &str, wasm: &str) -> PathBuf {
    let target = scratch.path().join(name);
    omnia::compile::compile(
        Path::new(wasm),
        Some(target.clone()),
        None,
        &CompileOptions::default(),
    )
    .unwrap_or_else(|error| panic!("compiling {wasm}: {error:#}"));
    target
}

// a wasm-pkg `local` backend, served by the registry `registry.test`
fn stage_package(root: &Path, package: &str, wasm: &str) {
    let (name, version) = package.split_once('@').expect("test packages pin versions");
    let (namespace, name) = name.split_once(':').expect("test packages are namespaced");
    let dir = root.join(namespace).join(name);
    std::fs::create_dir_all(&dir).expect("creating package directory");
    std::fs::copy(wasm, dir.join(format!("{version}.wasm"))).expect("staging package");
}

// serves `registry.test` from the `local` backend at `root`, routing nothing to it
fn local_backend_toml(root: &Path) -> String {
    format!(
        "[registry.\"registry.test\"]\ndefault = \"local\"\n\n[registry.\"registry.test\".local]\nroot \
         = {:?}\n",
        root.display().to_string()
    )
}

// routes every package to the `local` backend at `root`
fn local_registry_toml(root: &Path) -> String {
    format!("default_registry = \"registry.test\"\n\n{}", local_backend_toml(root))
}

fn echoer_package() -> GuestEntry {
    GuestEntry::package(ECHOER_PACKAGE)
}

// The declared path source: a path guest is absent until its first use —
// here the requester naming `plugin` — when the deployment admits the file
// it declared for that name and the registry records the bytes' digest. The
// requester is a path guest too, loaded when the command drive names it.
#[tokio::test]
async fn plugins_load() {
    let manifest = requester(test_programs::PLUGINS_LOAD)
        .guest(GuestEntry::new("plugin", test_programs::LINK_ECHOER));
    let runtime = boot(manifest, &["declared", "plugin"]).await.expect("assembling runtime");
    assert_eq!(runtime.registry().len(), 0, "path guests are absent until their first use");
    assert_eq!(recorded(&runtime, "plugin"), Registered::Absent);

    let status = runtime.run_command().await.expect("deployment runs");
    assert_eq!(status, ExitStatus::SUCCESS, "the requester's assertions all held");
    assert_eq!(
        recorded(&runtime, "requester"),
        Registered::Active(digest_of(test_programs::PLUGINS_LOAD)),
        "the command drive loaded the requester"
    );
    assert_eq!(
        recorded(&runtime, "plugin"),
        Registered::Active(digest_of(test_programs::LINK_ECHOER)),
        "the load recorded the admitted bytes' digest"
    );
    runtime.shutdown();
}

// `omnia compile` output loads from a path once the entry pins the bytes —
// the requester at the command drive, the plugin at the declared load — and
// the registry records its bytes' digest the same way.
#[tokio::test]
async fn precompiled_sources() {
    let scratch = scratch();
    let requester_bin = precompile(&scratch, "requester.bin", test_programs::PLUGINS_LOAD);
    let plugin_bin = precompile(&scratch, "plugin.bin", test_programs::LINK_ECHOER);
    let requester_digest = digest_of(requester_bin.to_str().expect("a utf-8 path"));
    let digest = digest_of(plugin_bin.to_str().expect("a utf-8 path"));

    let manifest = Manifest::new()
        .guest(GuestEntry::new("requester", requester_bin).digest(requester_digest).command())
        .guest(GuestEntry::new("plugin", plugin_bin).digest(digest));
    let runtime = boot(manifest, &["declared", "plugin"]).await.expect("assembling runtime");
    let status = runtime.run_command().await.expect("deployment runs");
    assert_eq!(status, ExitStatus::SUCCESS, "the requester's assertions all held");
    assert_eq!(recorded(&runtime, "requester"), Registered::Active(requester_digest));
    assert_eq!(
        recorded(&runtime, "plugin"),
        Registered::Active(digest),
        "the load recorded the compiled bytes' digest"
    );
    runtime.shutdown();
}

// A declared digest binds the name to exactly those bytes; the handle
// reports it back.
#[tokio::test]
async fn pinned() {
    let digest = digest_of(test_programs::LINK_ECHOER);
    let manifest = requester(test_programs::PLUGINS_LOAD)
        .guest(GuestEntry::new("plugin", test_programs::LINK_ECHOER).digest(digest));
    let status =
        run(manifest, &["declared", "plugin", &digest.to_string()]).await.expect("deployment runs");
    assert_eq!(status, ExitStatus::SUCCESS, "the requester's assertions all held");
}

#[tokio::test]
async fn pin_mismatch() {
    let manifest = requester(test_programs::PLUGINS_REFUSED).guest(
        GuestEntry::new("plugin", test_programs::LINK_ECHOER).digest(Digest::of(b"other bytes")),
    );
    let runtime = boot(manifest, &["declared", "plugin", "refused", "not its declared digest"])
        .await
        .expect("assembling runtime");
    let status = runtime.run_command().await.expect("deployment runs");
    assert_eq!(status, ExitStatus::SUCCESS, "the requester's assertions all held");
    assert_eq!(recorded(&runtime, "plugin"), Registered::Absent, "nothing was admitted");
    runtime.shutdown();
}

// What a pre-compiled artifact may load from follows from the source kind
// alone, no key: embedded bytes were in the process before any guest ran and
// load at boot; a path is read while guests run, so it loads only pinned
// (`precompiled_sources`) and is refused unpinned; a package admits raw wasm
// alone, however it hashes. The same artifact runs every row.
#[tokio::test]
async fn precompiled_by_source() {
    let scratch = scratch();
    let plugin_bin = precompile(&scratch, "plugin.bin", test_programs::LINK_ECHOER);
    let bytes = std::fs::read(&plugin_bin).expect("reading the compiled echoer");
    let pinned = Digest::of(&bytes);
    stage_package(scratch.path(), ECHOER_PACKAGE, plugin_bin.to_str().expect("a utf-8 path"));

    let manifest = requester(test_programs::PLUGINS_LOAD).guest(GuestEntry::new("plugin", bytes));
    let runtime = boot(manifest, &["declared", "plugin"]).await.expect("assembling runtime");
    assert_eq!(
        recorded(&runtime, "plugin"),
        Registered::Active(pinned),
        "embedded pre-compiled bytes loaded at boot"
    );
    let status = runtime.run_command().await.expect("deployment runs");
    assert_eq!(status, ExitStatus::SUCCESS, "the declared load attested the boot guest");
    runtime.shutdown();

    let rows = [
        ("unpinned path", GuestEntry::new("plugin", plugin_bin), "plugin", "unpinned", None),
        (
            "pinned package",
            echoer_package().digest(pinned),
            ECHOER_NAME,
            "a package admits raw wasm alone",
            Some(RegistryConfig::contents(local_registry_toml(scratch.path()))),
        ),
    ];
    for (row, entry, name, needle, registries) in rows {
        let mut manifest = requester(test_programs::PLUGINS_REFUSED).guest(entry);
        if let Some(registries) = registries {
            manifest = manifest.registries(registries);
        }
        let runtime = boot(manifest, &["declared", name, "refused", needle])
            .await
            .expect("assembling runtime");
        let status = runtime.run_command().await.expect("deployment runs");
        assert_eq!(status, ExitStatus::SUCCESS, "row `{row}`: the requester's assertions held");
        assert_eq!(recorded(&runtime, name), Registered::Absent, "row `{row}`: nothing admitted");
        runtime.shutdown();
    }
}

// The declared arm reads a path entry's `source.path` at its first use —
// when the requester asks — so a guest holding a writable mount over that
// path chooses what the host is handed. Pre-compiled bytes it plants there
// are refused as unpinned, where the raw wasm it plants loads: the host
// compiles that itself. The host reads the planted artifact back, so it is
// the refusal that kept it out, not a copy that never landed.
#[tokio::test]
async fn plugins_plant() {
    let scratch = scratch();
    precompile(&scratch, "payload.bin", test_programs::LINK_ECHOER);
    stage(&scratch, "payload.wasm", test_programs::LINK_ECHOER);
    let planted = scratch.path().join("plugin.bin");

    let manifest = requester(test_programs::PLUGINS_PLANT)
        .guest(GuestEntry::new("plugin", planted.clone()))
        .mounts([scratch.mount(true)]);
    let runtime = boot(manifest, &["payload.bin", "plugin.bin", "refused", "unpinned"])
        .await
        .expect("assembling runtime");
    let status = runtime.run_command().await.expect("deployment runs");
    assert_eq!(status, ExitStatus::SUCCESS, "the requester's assertions all held");
    assert_eq!(recorded(&runtime, "plugin"), Registered::Absent, "nothing was admitted");
    runtime.shutdown();
    let planted = std::fs::read(&planted).expect("the guest planted the artifact");
    assert!(Engine::detect_precompiled(&planted).is_some(), "what it planted is pre-compiled");

    let manifest = requester(test_programs::PLUGINS_PLANT)
        .guest(GuestEntry::new("plugin", scratch.path().join("plugin.wasm")))
        .mounts([scratch.mount(true)]);
    let runtime = boot(manifest, &["payload.wasm", "plugin.wasm", "loaded"])
        .await
        .expect("assembling runtime");
    let status = runtime.run_command().await.expect("deployment runs");
    assert_eq!(status, ExitStatus::SUCCESS, "the requester's assertions all held");
    assert_eq!(
        recorded(&runtime, "plugin"),
        Registered::Active(digest_of(test_programs::LINK_ECHOER)),
        "the planted raw wasm loaded"
    );
    runtime.shutdown();
}

// A package source is fetched at its first use from the registry the
// manifest's `registries` routes it to, and registers under its declared
// name — the reference without its version, never the reference itself.
#[tokio::test]
async fn plugins_load_package() {
    let scratch = scratch();
    stage_package(scratch.path(), ECHOER_PACKAGE, test_programs::LINK_ECHOER);
    let manifest = requester(test_programs::PLUGINS_LOAD)
        .guest(echoer_package())
        .registries(RegistryConfig::contents(local_registry_toml(scratch.path())));

    let runtime = boot(manifest, &["declared", ECHOER_NAME]).await.expect("assembling runtime");
    assert_eq!(recorded(&runtime, ECHOER_NAME), Registered::Absent, "not fetched until used");
    let status = runtime.run_command().await.expect("deployment runs");
    assert_eq!(status, ExitStatus::SUCCESS, "the requester's assertions all held");
    assert_eq!(
        recorded(&runtime, ECHOER_NAME),
        Registered::Active(digest_of(test_programs::LINK_ECHOER)),
        "the package registered under its declared name"
    );
    assert_eq!(recorded(&runtime, ECHOER_PACKAGE), Registered::Absent);
    runtime.shutdown();
}

// An embedder's registry source replaces the declared routing wholesale: the
// manifest declares no `registries`, and the selected client serves the
// package.
#[tokio::test]
async fn custom_registry_source() {
    let scratch = scratch();
    stage_package(scratch.path(), ECHOER_PACKAGE, test_programs::LINK_ECHOER);
    let client = RegistryClient::from_toml(&local_registry_toml(scratch.path()))
        .expect("the local registry configuration parses");

    let manifest = requester(test_programs::PLUGINS_LOAD).guest(echoer_package());
    let mut deployment =
        deployment(manifest, &["declared", ECHOER_NAME]).await.expect("building deployment");
    deployment.registry_source(client);
    let runtime =
        deployment.assemble(Backends::defaults().await).await.expect("assembling runtime");
    let status = runtime.run_command().await.expect("deployment runs");
    assert_eq!(status, ExitStatus::SUCCESS, "the requester's assertions all held");
    runtime.shutdown();
}

// A package the registry has no release of refuses: retrying the same
// reference cannot succeed.
#[tokio::test]
async fn package_not_found() {
    let scratch = scratch();
    let manifest = requester(test_programs::PLUGINS_REFUSED)
        .guest(echoer_package())
        .registries(RegistryConfig::contents(local_registry_toml(scratch.path())));
    let status =
        run(manifest, &["declared", ECHOER_NAME, "refused", "resolving `test:echoer@1.0.0`"])
            .await
            .expect("deployment runs");
    assert_eq!(status, ExitStatus::SUCCESS, "the requester's assertions all held");
}

// The refusal matrix, one requester run per row: the deployment declares
// (or does not declare) a name, and the wire carries the variant and the
// cause. Nothing a row names is ever admitted.
#[tokio::test]
async fn plugins_refused() {
    let scratch = scratch();
    let junk = scratch.path().join("junk.wasm");
    std::fs::write(&junk, b"not a component").expect("staging junk");
    let absent = scratch.path().join("absent.wasm");

    let rows: [(&str, GuestEntry, &str, &str); 4] = [
        ("junk", GuestEntry::new("junk", junk), "refused", "validating `junk`"),
        ("absent", GuestEntry::new("absent", absent), "unavailable", "reading"),
        // no `registries` at all: nothing routes any package
        (ECHOER_NAME, echoer_package(), "refused", "no registry routes `test:echoer`"),
        // a guest declared under another name is still just a name
        (
            "nonesuch",
            GuestEntry::new("other", test_programs::LINK_ECHOER),
            "refused",
            "no guest `nonesuch` is declared",
        ),
    ];
    for (name, entry, variant, needle) in rows {
        let manifest = requester(test_programs::PLUGINS_REFUSED).guest(entry);
        let runtime =
            boot(manifest, &["declared", name, variant, needle]).await.expect("assembling runtime");
        let status = runtime.run_command().await.expect("deployment runs");
        assert_eq!(status, ExitStatus::SUCCESS, "row `{name}`: the requester's assertions held");
        assert_eq!(
            recorded(&runtime, name),
            Registered::Absent,
            "row `{name}`: nothing was admitted"
        );
        runtime.shutdown();
    }
}

// A `registries` configuration that routes nothing for a package's namespace
// refuses the load, naming the namespace, rather than reaching a fallback.
#[tokio::test]
async fn unrouted_namespace() {
    let manifest = requester(test_programs::PLUGINS_REFUSED)
        .guest(echoer_package())
        .registries(RegistryConfig::contents("[namespace_registries]\nwasi = \"wasi.dev\"\n"));
    let status = run(manifest, &["declared", ECHOER_NAME, "refused", "`test` namespace"])
        .await
        .expect("deployment runs");
    assert_eq!(status, ExitStatus::SUCCESS, "the requester's assertions all held");
}

// A path the requester names loads through the read-only mount it lies
// beneath — `.` for a bare relative path, a mount's name as its prefix
// otherwise — registers as its file stem, and records the bytes' digest.
#[tokio::test]
async fn plugins_load_path() {
    let scratch = scratch();
    stage(&scratch, "adapters/plugin.wasm", test_programs::LINK_ECHOER);
    let manifest = requester(test_programs::PLUGINS_LOAD).mounts([scratch.mount(false)]);
    let runtime =
        boot(manifest, &["path", "./adapters/plugin.wasm"]).await.expect("assembling runtime");
    let status = runtime.run_command().await.expect("deployment runs");
    assert_eq!(status, ExitStatus::SUCCESS, "the requester's assertions all held");
    assert_eq!(
        recorded(&runtime, "plugin"),
        Registered::Active(digest_of(test_programs::LINK_ECHOER)),
        "the load recorded the read bytes' digest under the file stem"
    );
    runtime.shutdown();

    let manifest = requester(test_programs::PLUGINS_LOAD).mounts([scratch.mount_as("code", false)]);
    let status =
        run(manifest, &["path", "code/adapters/plugin.wasm"]).await.expect("deployment runs");
    assert_eq!(status, ExitStatus::SUCCESS, "a named mount is addressed by its name");
}

// A path whose stem is an active guest attests it when the bytes are the
// same: the deployment booted the echoer from embedded bytes, and the
// requester names its file.
#[tokio::test]
async fn path_attests_active() {
    let scratch = scratch();
    stage(&scratch, "echoer.wasm", test_programs::LINK_ECHOER);
    let echoer = std::fs::read(test_programs::LINK_ECHOER).expect("reading the echoer");
    let manifest = requester(test_programs::PLUGINS_LOAD)
        .guest(GuestEntry::new("echoer", echoer))
        .mounts([scratch.mount(false)]);
    let digest = digest_of(test_programs::LINK_ECHOER).to_string();
    let status = run(manifest, &["path", "echoer.wasm", &digest]).await.expect("deployment runs");
    assert_eq!(status, ExitStatus::SUCCESS, "the requester's assertions all held");
}

// A path or package whose derived name the deployment declares is refused
// before anything is read, whether or not that guest has loaded: the name
// is bound by its entry alone, so a caller cannot seat other bytes under it
// for the declared load to attest. A package derives its name without the
// version, so another release of a declared package is refused the same.
#[tokio::test]
async fn declared_name_reserved() {
    let scratch = scratch();
    scratch.write("plugin.wasm", changed_echoer(b"squatter"));
    stage_package(scratch.path(), ECHOER_PACKAGE, test_programs::LINK_ECHOER);

    for row in [
        ["path", "plugin.wasm", "refused", "declares"],
        ["registry", ECHOER_PACKAGE, "refused", "declares"],
        ["registry", "test:echoer@9.9.9", "refused", "declares"],
    ] {
        let manifest = requester(test_programs::PLUGINS_REFUSED)
            .guest(GuestEntry::new("plugin", test_programs::LINK_ECHOER))
            .guest(GuestEntry::new(ECHOER_NAME, test_programs::LINK_ECHOER))
            .mounts([scratch.mount(false)])
            .registries(RegistryConfig::contents(local_registry_toml(scratch.path())));
        let runtime = boot(manifest, &row).await.expect("assembling runtime");
        let status = runtime.run_command().await.expect("deployment runs");
        assert_eq!(status, ExitStatus::SUCCESS, "row {row:?}: the requester's assertions held");
        assert_eq!(runtime.registry().len(), 1, "row {row:?}: only the requester loaded");
        runtime.shutdown();
    }
}

// A package the requester names is fetched from the registry the
// deployment's `registries` routes it to, and registers as its reference
// without the version.
#[tokio::test]
async fn plugins_load_registry() {
    let scratch = scratch();
    stage_package(scratch.path(), ECHOER_PACKAGE, test_programs::LINK_ECHOER);
    let manifest = requester(test_programs::PLUGINS_LOAD)
        .registries(RegistryConfig::contents(local_registry_toml(scratch.path())));
    let runtime = boot(manifest, &["registry", ECHOER_PACKAGE]).await.expect("assembling runtime");
    let status = runtime.run_command().await.expect("deployment runs");
    assert_eq!(status, ExitStatus::SUCCESS, "the requester's assertions all held");
    assert_eq!(
        recorded(&runtime, ECHOER_NAME),
        Registered::Active(digest_of(test_programs::LINK_ECHOER)),
        "the package registered under its unversioned reference"
    );
    assert_eq!(recorded(&runtime, ECHOER_PACKAGE), Registered::Absent);
    runtime.shutdown();
}

// The refusal matrix over requester-named locations, one run per row, over
// a deployment mounting the scratch directory read-only as `.`: a path that
// escapes, a missing file, junk, a pre-compiled artifact, a malformed or
// mismatched pin, a stem the deployment declares (the requester itself,
// loaded and running), and a declared name pinned by the call. Nothing a
// row names is ever admitted — the requester stays the registry's one
// guest.
#[tokio::test]
async fn location_refused() {
    let scratch = scratch();
    stage(&scratch, "plugin.wasm", test_programs::LINK_ECHOER);
    scratch.write("junk.wasm", b"not a component");
    scratch.write("requester.wasm", changed_echoer(b"impostor"));
    precompile(&scratch, "native.bin", test_programs::LINK_ECHOER);
    let other = Digest::of(b"other bytes").to_string();

    let rows: [&[&str]; 9] = [
        &["path", "../plugin.wasm", "refused", "plain relative path"],
        &["path", "/etc/passwd", "refused", "plain relative path"],
        &["path", "absent.wasm", "unavailable", "reading component"],
        &["path", "junk.wasm", "refused", "validating `junk`"],
        &["path", "native.bin", "refused", "admits raw wasm alone"],
        &["path", "plugin.wasm", "refused", "digest `sha256:zz`", "sha256:zz"],
        &["path", "plugin.wasm", "refused", "not its declared digest", &other],
        &["path", "requester.wasm", "refused", "declares"],
        &["declared", "requester", "refused", "takes no digest", &other],
    ];
    for row in rows {
        let manifest = requester(test_programs::PLUGINS_REFUSED).mounts([scratch.mount(false)]);
        let runtime = boot(manifest, row).await.expect("assembling runtime");
        let status = runtime.run_command().await.expect("deployment runs");
        assert_eq!(status, ExitStatus::SUCCESS, "row {row:?}: the requester's assertions held");
        assert_eq!(runtime.registry().len(), 1, "row {row:?}: nothing was admitted");
        runtime.shutdown();
    }

    // A stem active under other bytes, where no entry declares the name: the
    // echoer booted from embedded bytes, and the file planted under its name
    // is refused rather than re-bound (the same bytes would attest —
    // `path_attests_active`).
    scratch.write("echoer.wasm", changed_echoer(b"impostor"));
    let echoer = std::fs::read(test_programs::LINK_ECHOER).expect("reading the echoer");
    let manifest = requester(test_programs::PLUGINS_REFUSED)
        .guest(GuestEntry::new("echoer", echoer))
        .mounts([scratch.mount(false)]);
    let row = ["path", "echoer.wasm", "refused", "active under other bytes"];
    let runtime = boot(manifest, &row).await.expect("assembling runtime");
    let status = runtime.run_command().await.expect("deployment runs");
    assert_eq!(status, ExitStatus::SUCCESS, "the requester's assertions all held");
    assert_eq!(runtime.registry().len(), 2, "nothing was admitted beside the two guests");
    runtime.shutdown();

    // With no mount at all, no path resolves.
    let manifest = requester(test_programs::PLUGINS_REFUSED);
    let status = run(manifest, &["path", "plugin.wasm", "refused", "beneath no mount"])
        .await
        .expect("deployment runs");
    assert_eq!(status, ExitStatus::SUCCESS, "the requester's assertions all held");
}

// A component beneath a writable mount never loads, whether the mount is
// `.` or the path names it: what the guest can write, it cannot run. The
// refusal names the mount and is reached before the file is read.
#[tokio::test]
async fn writable_mount_refused() {
    let scratch = scratch();
    stage(&scratch, "plugin.wasm", test_programs::LINK_ECHOER);
    let manifest = requester(test_programs::PLUGINS_REFUSED).mounts([scratch.mount(true)]);
    let status = run(manifest, &["path", "plugin.wasm", "refused", "writable mount `.`"])
        .await
        .expect("deployment runs");
    assert_eq!(status, ExitStatus::SUCCESS, "a writable `.` serves no load");

    let code = omnia_test::host::scratch();
    let manifest = requester(test_programs::PLUGINS_REFUSED)
        .mounts([code.mount(false), scratch.mount_as("out", true)]);
    let status = run(manifest, &["path", "out/plugin.wasm", "refused", "writable mount `out`"])
        .await
        .expect("deployment runs");
    assert_eq!(status, ExitStatus::SUCCESS, "a writable mount named by prefix serves no load");
}

// A writable mount that shares or nests a read-only mount's directory
// refuses at assembly, naming both: written through one view, a component
// would load through the other.
#[tokio::test]
async fn overlap_refused_at_install() {
    let scratch = scratch();
    std::fs::create_dir(scratch.path().join("out")).expect("creating out");
    let out = |writable| omnia::Mount {
        name: "out".to_owned(),
        path: scratch.path().join("out"),
        writable,
    };

    for mounts in [
        vec![scratch.mount(false), out(true)],
        vec![scratch.mount(true), out(false)],
        vec![scratch.mount(false), scratch.mount_as("shared", true)],
    ] {
        let manifest = requester(test_programs::PLUGINS_LOAD).mounts(mounts);
        let Err(error) = boot(manifest, &[]).await else {
            panic!("the overlap is refused");
        };
        let detail = format!("{error:#}");
        assert!(detail.contains("writable mount"), "{detail}");
        assert!(detail.contains("read-only mount"), "{detail}");
    }

    let manifest =
        requester(test_programs::PLUGINS_LOAD).mounts([scratch.mount(false), out(false)]);
    boot(manifest, &[]).await.expect("two read-only mounts may nest").shutdown();
}

// The deployment's `registries` outranks the registry a load names: a
// package whose namespace it routes — here through the default — is refused
// rather than fetched elsewhere, and the refusal names both registries.
#[tokio::test]
async fn endpoint_conflicts_with_routing() {
    let scratch = scratch();
    stage_package(scratch.path(), ECHOER_PACKAGE, test_programs::LINK_ECHOER);
    let manifest = requester(test_programs::PLUGINS_REFUSED)
        .registries(RegistryConfig::contents(local_registry_toml(scratch.path())));
    let status = run(
        manifest,
        &["registry@other.test", ECHOER_PACKAGE, "refused", "routed to `registry.test`"],
    )
    .await
    .expect("deployment runs");
    assert_eq!(status, ExitStatus::SUCCESS, "the requester's assertions all held");
}

// A namespace the deployment routes nowhere is fetched from the registry the
// load names — one the deployment's `registries` configures but routes
// nothing to.
#[tokio::test]
async fn endpoint_unrouted_namespace() {
    let scratch = scratch();
    stage_package(scratch.path(), ECHOER_PACKAGE, test_programs::LINK_ECHOER);
    let manifest = requester(test_programs::PLUGINS_LOAD)
        .registries(RegistryConfig::contents(local_backend_toml(scratch.path())));
    let status =
        run(manifest, &["registry@registry.test", ECHOER_PACKAGE]).await.expect("deployment runs");
    assert_eq!(status, ExitStatus::SUCCESS, "the requester's assertions all held");
}

// Loading a name that is already active attests it — a boot guest from
// embedded bytes, the requester the command drive loaded — with the digest
// its registration recorded for its bytes.
#[tokio::test]
async fn plugins_attest() {
    let bytes = std::fs::read(test_programs::LINK_ECHOER).expect("reading the echoer");
    let manifest = requester(test_programs::PLUGINS_ATTEST).guest(GuestEntry::new("echoer", bytes));
    let echoer = digest_of(test_programs::LINK_ECHOER).to_string();
    let runtime = boot(manifest.clone(), &["echoer", &echoer]).await.expect("assembling runtime");
    assert_eq!(recorded(&runtime, "echoer"), Registered::Active(echoer.parse().expect("digest")));
    let status = runtime.run_command().await.expect("deployment runs");
    assert_eq!(status, ExitStatus::SUCCESS, "a boot guest attests with its digest");
    runtime.shutdown();
    let requester = digest_of(test_programs::PLUGINS_ATTEST).to_string();
    let status = run(manifest, &["requester", &requester]).await.expect("deployment runs");
    assert_eq!(status, ExitStatus::SUCCESS, "the requester attests itself");
}

// Admission does not require a linked export: a component exporting no
// `omnia-test:link/ops` loads (it stays reachable through the host
// `Dispatcher`), and a link call to it fails at the call site — the polyfill
// traps the requester, so the failure is observed here, after the load.
#[tokio::test]
async fn plugins_unlinked() {
    let manifest = requester(test_programs::PLUGINS_UNLINKED)
        .guest(GuestEntry::new("noseam", test_programs::LINK_FULL));
    let runtime = boot(manifest, &[]).await.expect("assembling runtime");

    let error = runtime.run_command().await.expect_err("the link call traps the requester");
    assert!(
        recorded(&runtime, "noseam") != Registered::Absent,
        "the load succeeded before the call failed: {error:#}"
    );
    let detail = format!("{error:#}");
    assert!(
        detail.contains("guest `noseam` is registered but exports no linked interface"),
        "the trap diagnoses the unlinked target: {detail}"
    );
    runtime.shutdown();
}

// Plugin without link: the requester imports no `ops`, loads a host-only
// handler and exits, then the host drives it through the Dispatcher.
#[tokio::test]
async fn plugins_host_only() {
    let manifest = requester(test_programs::PLUGINS_HOST_ONLY)
        .guest(GuestEntry::new("plugin", test_programs::LINK_ECHOER));
    let runtime = boot(manifest, &[]).await.expect("assembling runtime");

    let status = runtime.run_command().await.expect("deployment runs");
    assert_eq!(status, ExitStatus::SUCCESS, "the requester's load held");
    assert_ne!(
        recorded(&runtime, "plugin"),
        Registered::Absent,
        "the host-only handler is registered"
    );

    let answer = invoke_ping(&runtime, "plugin", "hi").await.expect("host dispatch");
    assert_eq!(answer, "plugin pong: hi");
    runtime.shutdown();
}

// Link + plugin, mixed: one loaded plugin exports the linked interface and
// answers a guest call; another exports nothing linked, loads fine, and is
// driven host-side. Both loads succeed — no admission check.
#[tokio::test]
async fn plugins_mixed() {
    let manifest = requester(test_programs::PLUGINS_MIXED)
        .guest(GuestEntry::new("echoer", test_programs::LINK_ECHOER))
        .guest(GuestEntry::new("handler", test_programs::LINK_FULL));
    let runtime = boot(manifest, &[]).await.expect("assembling runtime");

    let status = runtime.run_command().await.expect("deployment runs");
    assert_eq!(status, ExitStatus::SUCCESS, "the requester's assertions all held");
    assert_ne!(recorded(&runtime, "echoer"), Registered::Absent, "the link target is registered");
    assert_ne!(
        recorded(&runtime, "handler"),
        Registered::Absent,
        "the host-only handler is registered"
    );

    let answer =
        call_export(&runtime, "handler", "poke", "hi").await.expect("host dispatch of the handler");
    assert_eq!(answer, "echoer pong: hi");
    runtime.shutdown();
}

// Appending a custom section changes a component's bytes, and digest,
// without changing its behavior. Single-byte LEB128 sizes, so keep it short.
fn custom_section(payload: &[u8]) -> Vec<u8> {
    let name = b"omnia-test";
    let mut body = vec![u8::try_from(name.len()).expect("short name")];
    body.extend_from_slice(name);
    body.extend_from_slice(payload);
    let mut section = vec![0x00, u8::try_from(body.len()).expect("short section")];
    section.extend_from_slice(&body);
    section
}

// same behavior, new digest
fn changed_echoer(payload: &[u8]) -> Vec<u8> {
    let mut changed = std::fs::read(test_programs::LINK_ECHOER).expect("reading the echoer");
    changed.extend_from_slice(&custom_section(payload));
    changed
}

// Host-side loads over the same runtime the guests drive: deregistration is
// host authority, so the WASI surface cannot reach this scenario. A path
// source is read fresh on every admission, so the re-load binds the freshly
// staged bytes and records their digest.
#[tokio::test]
async fn reload_after_deregister() {
    let scratch = scratch();
    let staged = stage(&scratch, "plugin.wasm", test_programs::LINK_ECHOER);
    // The requester guest is manifest ballast: these loads are host-driven.
    let manifest =
        requester(test_programs::PLUGINS_LOAD).guest(GuestEntry::new("plugin", staged.clone()));
    let runtime = boot(manifest, &[]).await.expect("assembling runtime");

    let first = runtime.load(declared("plugin"), None).await.expect("first load");
    runtime.deregister(first.id()).expect("deregistering the loaded plugin");
    assert_eq!(recorded(&runtime, "plugin"), Registered::Absent);

    let changed = changed_echoer(b"reload");
    std::fs::write(&staged, &changed).expect("re-staging");
    let fresh = runtime.load(declared("plugin"), None).await.expect("re-load");
    assert_ne!(fresh.digest(), first.digest(), "the re-load bound fresh bytes");
    assert_eq!(fresh.digest(), Digest::of(&changed));
    assert_eq!(recorded(&runtime, "plugin"), Registered::Active(fresh.digest()));
    runtime.shutdown();
}

// The declared digest outlives any one registration: once the staged file
// changes, a re-load refuses rather than binding the name to new bytes.
#[tokio::test]
async fn pinned_reload() {
    let scratch = scratch();
    let staged = stage(&scratch, "plugin.wasm", test_programs::LINK_ECHOER);
    let manifest = requester(test_programs::PLUGINS_LOAD).guest(
        GuestEntry::new("plugin", staged.clone()).digest(digest_of(test_programs::LINK_ECHOER)),
    );
    let runtime = boot(manifest, &[]).await.expect("assembling runtime");

    let first = runtime.load(declared("plugin"), None).await.expect("the pinned bytes load");
    runtime.deregister(first.id()).expect("deregistering the loaded plugin");
    std::fs::write(&staged, changed_echoer(b"pinned")).expect("re-staging");

    let stale =
        runtime.load(declared("plugin"), None).await.expect_err("the staged bytes miss the pin");
    assert!(
        matches!(&stale, LoadError::Refused(detail) if detail.contains("not its declared digest")),
        "{stale:?}"
    );
    assert_eq!(recorded(&runtime, "plugin"), Registered::Absent, "nothing was admitted");
    runtime.shutdown();
}

// An identity the embedder re-registered outside the load path is active, so
// a load attests it as it stands — with the digest `Runtime::register`
// recorded for the re-registered bytes — rather than re-binding or
// re-admitting it from the declared source.
#[tokio::test]
async fn reregister_attests() {
    let manifest = requester(test_programs::PLUGINS_LOAD)
        .guest(GuestEntry::new("plugin", test_programs::LINK_ECHOER));
    let runtime = boot(manifest, &[]).await.expect("assembling runtime");

    let first = runtime.load(declared("plugin"), None).await.expect("first load");
    runtime.deregister(first.id()).expect("deregistering the loaded plugin");
    let reregistered = changed_echoer(b"reregister");
    runtime.register("plugin", reregistered.clone()).await.expect("re-registering externally");

    let attested =
        runtime.load(declared("plugin"), None).await.expect("an active identity attests");
    assert_eq!(attested.id(), first.id());
    assert_eq!(
        attested.digest(),
        Digest::of(&reregistered),
        "the embedder's registration recorded its bytes' digest"
    );
    assert_ne!(attested.digest(), first.digest(), "the declared source was not re-admitted");
    runtime.shutdown();
}

// The guest-side copies exist because published crates cannot reference WIT
// outside their package root; the host copy stays canonical.
#[test]
fn wit_copies_stay_identical() {
    let canonical = include_str!("../wit/plugins.wit");
    assert_eq!(
        include_str!("../../omnia-sdk/wit/plugins.wit"),
        canonical,
        "omnia-sdk's plugins.wit copy drifted from crates/omnia-plugin/wit/plugins.wit"
    );
    assert_eq!(
        include_str!("../../test-programs/wit/deps/plugins/plugins.wit"),
        canonical,
        "test-programs' plugins.wit copy drifted from crates/omnia-plugin/wit/plugins.wit"
    );
}

// Compile-time proof that the macro's inline keys lower into the manifest
// data this crate consumes: a `package` guest with a `digest`, named by its
// reference, and the `registries` contents that route it. The macro's
// snapshot suite pins the expansion shape; this pins the types and the
// carried data.
mod loader_grammar {
    omnia::runtime!({
        guests: [
            {
                package: "acme:tool@1.2.3",
                digest: "sha256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
            },
        ],
        registries: "default_registry = \"ghcr.io\"\n",
    });
}

#[test]
fn loader_grammar() {
    // Touch the generated entry points so the compile-only module above is
    // reachable for dead-code analysis.
    let _ = (loader_grammar::main, loader_grammar::run, loader_grammar::run_with::<()>);
    let manifest = loader_grammar::manifest().into_manifest().expect("inline source resolves");
    assert_eq!(
        manifest.registries,
        Some(RegistryConfig::contents("default_registry = \"ghcr.io\"\n"))
    );
    let [tool] = manifest.guests.as_slice() else {
        panic!("one guest: {:?}", manifest.guests);
    };
    assert_eq!(tool.name, "acme:tool", "the reference without its version");
    assert_eq!(tool.digest, Some(Digest::of(b"")), "the empty input's digest, as written");
    assert!(matches!(&tool.source, SourceSpec::Package(package) if package == "acme:tool@1.2.3"));
}

// `RegistrySource` stays the seam an embedder fills: a source of its own is
// reached exactly as the built-in client is.
#[tokio::test]
async fn custom_registry_trait() {
    struct Fixed(Vec<u8>);

    impl RegistrySource for Fixed {
        fn acquire<'a>(
            &'a self, package: &'a str, endpoint: Option<&'a str>,
        ) -> omnia::futures::future::BoxFuture<'a, Result<Vec<u8>, AcquireError>> {
            Box::pin(async move {
                assert_eq!(package, ECHOER_PACKAGE);
                assert_eq!(endpoint, None, "a declared package names no registry of its own");
                Ok(self.0.clone())
            })
        }
    }

    let bytes = std::fs::read(test_programs::LINK_ECHOER).expect("reading the echoer");
    let manifest = requester(test_programs::PLUGINS_LOAD).guest(echoer_package());
    let mut deployment =
        deployment(manifest, &["declared", ECHOER_NAME]).await.expect("building deployment");
    deployment.registry_source(Fixed(bytes));
    let runtime =
        deployment.assemble(Backends::defaults().await).await.expect("assembling runtime");
    let status = runtime.run_command().await.expect("deployment runs");
    assert_eq!(status, ExitStatus::SUCCESS, "the requester's assertions all held");
    runtime.shutdown();
}
