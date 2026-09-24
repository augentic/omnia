//! End-to-end tests for the `omnia:plugins/loader` host capability: a real
//! requester guest from `crates/test-programs` drives loads through omnia's
//! runtime against a deployment whose `[[guest]]` list declares what may be
//! loaded — from a staged file, embedded bytes, or a wasm-pkg `local`
//! registry. The requester asserts internally (handles, digests, dispatch
//! answers, and every typed refusal); the host side declares the deployment
//! and checks the exit and the registry. Lifecycle scenarios the WASI
//! surface cannot reach (deregistration, embedder re-registration) drive
//! [`PluginLoader`] host-side over the same runtime.

#![cfg(not(target_arch = "wasm32"))]

use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result, bail};
use omnia::wasmtime::component::Val;
use omnia::{
    ChainCtx, DeploymentBuilder, Digest, ExitStatus, GuestArtifact, GuestEntry, GuestId, LoadError,
    Manifest, Mode, PluginLoader as _, RegistryClient, RegistryConfig, RegistrySource, Runtime,
    SourceSpec, StoreCtx,
};
use omnia_test::host::{Backends, Scratch, scratch};
use omnia_wasi_otel::WasiOtel;

// Every guest program in `crates/test-programs/programs/plugins` must have a
// matching test here; a new program without one fails to compile.
test_programs::foreach_plugins!();

const ECHOER_PACKAGE: &str = "test:echoer@1.0.0";

/// A manifest booting `wasm` as the `requester` command guest; the tests
/// declare what it may load beside it.
fn requester(wasm: &str) -> Manifest {
    Manifest::new().guest(GuestEntry::new("requester", wasm))
}

/// A `[[guest]]` admitted on its first load from a path or embedded bytes.
fn on_demand(name: &str, source: impl Into<SourceSpec>) -> GuestEntry {
    GuestEntry::new(name, source).on_demand()
}

/// Build `manifest` in command mode with `args` as the guest's argv past the
/// program name, the telemetry host serving the `command!` guest's otel
/// imports; the loader host is assembly's. Nothing declares what the
/// requester imports: the seam is read off the components.
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

/// Assemble `manifest` over the declared policy: package sources fetch
/// through the manifest's `registries`.
async fn boot(manifest: Manifest, args: &[&str]) -> Result<Runtime<Backends>> {
    deployment(manifest, args)
        .await?
        .assemble(Backends::defaults().await)
        .await
        .context("assembling runtime")
}

/// Boot and drive the requester once.
async fn run(manifest: Manifest, args: &[&str]) -> Result<ExitStatus> {
    let runtime = boot(manifest, args).await?;
    let status = runtime.run_command().await;
    runtime.shutdown();
    status
}

/// Host→guest `omnia-test:link/ops` `ping` through the [`Dispatcher`], from a
/// server root.
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

/// Instantiate `guest` fresh and drive a world-level string export.
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

/// What the registry holds for a guest name.
#[derive(Debug, PartialEq, Eq)]
enum Registered {
    Absent,
    /// Active, with the digest recorded at admission (`None` if never hashed).
    Active(Option<Digest>),
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

/// Stage `wasm` in the scratch dir under `name`, returning the staged path.
fn stage(scratch: &Scratch, name: &str, wasm: &str) -> PathBuf {
    let target = scratch.path().join(name);
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent).expect("creating the staging directory");
    }
    std::fs::copy(wasm, &target).unwrap_or_else(|error| panic!("staging {name}: {error}"));
    target
}

/// Compile `wasm` ahead of time into the scratch dir under `name` — what
/// `omnia compile` writes — returning the artifact's path.
fn precompile(scratch: &Scratch, name: &str, wasm: &str) -> PathBuf {
    let target = scratch.path().join(name);
    omnia::compile::compile(Path::new(wasm), Some(target.clone()))
        .unwrap_or_else(|error| panic!("compiling {wasm}: {error:#}"));
    target
}

/// Stage `wasm` as `package` in a wasm-pkg `local` backend rooted at `root`,
/// served by the registry `registry.test`.
fn stage_package(root: &Path, package: &str, wasm: &str) {
    let (name, version) = package.split_once('@').expect("test packages pin versions");
    let (namespace, name) = name.split_once(':').expect("test packages are namespaced");
    let dir = root.join(namespace).join(name);
    std::fs::create_dir_all(&dir).expect("creating package directory");
    std::fs::copy(wasm, dir.join(format!("{version}.wasm"))).expect("staging package");
}

/// The `registries` TOML routing every package to the `local` backend at
/// `root` — what a manifest's `[registries]` file or the macro's
/// `registries:` would carry.
fn local_registry_toml(root: &Path) -> String {
    format!(
        "default_registry = \"registry.test\"\n\n[registry.\"registry.test\"]\ndefault = \
         \"local\"\n\n[registry.\"registry.test\".local]\nroot = {:?}\n",
        root.display().to_string()
    )
}

/// The on-demand declaration of the echoer as a registry package.
fn echoer_package() -> GuestEntry {
    GuestEntry::new("plugin", SourceSpec::package(ECHOER_PACKAGE)).on_demand()
}

// The declared path source: the requester names `plugin`, the deployment
// admits the file it declared for that name, and the registry records the
// bytes' digest.
#[tokio::test]
async fn plugins_load() {
    let manifest = requester(test_programs::PLUGINS_LOAD)
        .guest(on_demand("plugin", test_programs::LINK_ECHOER));
    let runtime = boot(manifest, &["plugin"]).await.expect("assembling runtime");
    assert_eq!(
        recorded(&runtime, "plugin"),
        Registered::Absent,
        "an on-demand guest is absent until loaded"
    );

    let status = runtime.run_command().await.expect("deployment runs");
    assert_eq!(status, ExitStatus::SUCCESS, "the requester's assertions all held");
    assert_eq!(
        recorded(&runtime, "plugin"),
        Registered::Active(Some(digest_of(test_programs::LINK_ECHOER))),
        "the load recorded the admitted bytes' digest"
    );
    runtime.shutdown();
}

// Embedded bytes are a source like any other, held for the first load.
#[tokio::test]
async fn embedded_source() {
    let bytes = std::fs::read(test_programs::LINK_ECHOER).expect("reading the echoer");
    let manifest = requester(test_programs::PLUGINS_LOAD).guest(on_demand("plugin", bytes));
    let status = run(manifest, &["plugin"]).await.expect("deployment runs");
    assert_eq!(status, ExitStatus::SUCCESS, "the requester's assertions all held");
}

// `omnia compile` output loads wherever raw wasm does — a boot guest and an
// on-demand one alike — and the registry records its bytes' digest the same
// way.
#[tokio::test]
async fn precompiled_sources() {
    let scratch = scratch();
    let requester_bin = precompile(&scratch, "requester.bin", test_programs::PLUGINS_LOAD);
    let plugin_bin = precompile(&scratch, "plugin.bin", test_programs::LINK_ECHOER);
    let digest = Digest::of(&std::fs::read(&plugin_bin).expect("reading the compiled echoer"));

    let manifest = Manifest::new()
        .guest(GuestEntry::new("requester", requester_bin))
        .guest(on_demand("plugin", plugin_bin));
    let runtime = boot(manifest, &["plugin"]).await.expect("assembling runtime");
    let status = runtime.run_command().await.expect("deployment runs");
    assert_eq!(status, ExitStatus::SUCCESS, "the requester's assertions all held");
    assert_eq!(
        recorded(&runtime, "plugin"),
        Registered::Active(Some(digest)),
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
        .guest(on_demand("plugin", test_programs::LINK_ECHOER).digest(digest));
    let status = run(manifest, &["plugin", &digest.to_string()]).await.expect("deployment runs");
    assert_eq!(status, ExitStatus::SUCCESS, "the requester's assertions all held");
}

#[tokio::test]
async fn pin_mismatch() {
    let manifest = requester(test_programs::PLUGINS_REFUSED)
        .guest(on_demand("plugin", test_programs::LINK_ECHOER).digest(Digest::of(b"other bytes")));
    let runtime = boot(manifest, &["plugin", "refused", "not its declared digest"])
        .await
        .expect("assembling runtime");
    let status = runtime.run_command().await.expect("deployment runs");
    assert_eq!(status, ExitStatus::SUCCESS, "the requester's assertions all held");
    assert_eq!(recorded(&runtime, "plugin"), Registered::Absent, "nothing was admitted");
    runtime.shutdown();
}

// A package source is fetched from the registry the manifest's `registries`
// routes it to, and registers under its declared name — never the package
// reference.
#[tokio::test]
async fn plugins_load_package() {
    let scratch = scratch();
    stage_package(scratch.path(), ECHOER_PACKAGE, test_programs::LINK_ECHOER);
    let manifest = requester(test_programs::PLUGINS_LOAD)
        .guest(echoer_package())
        .registries(RegistryConfig::contents(local_registry_toml(scratch.path())));

    let runtime = boot(manifest, &["plugin"]).await.expect("assembling runtime");
    let status = runtime.run_command().await.expect("deployment runs");
    assert_eq!(status, ExitStatus::SUCCESS, "the requester's assertions all held");
    assert_eq!(
        recorded(&runtime, "plugin"),
        Registered::Active(Some(digest_of(test_programs::LINK_ECHOER))),
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
    let mut deployment = deployment(manifest, &["plugin"]).await.expect("building deployment");
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
    let status = run(manifest, &["plugin", "refused", "resolving `test:echoer@1.0.0`"])
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
        ("junk", on_demand("junk", junk), "refused", "validating `junk`"),
        ("absent", on_demand("absent", absent), "unavailable", "reading"),
        // No `registries` at all: nothing routes any package.
        ("plugin", echoer_package(), "refused", "no registry routes `test:echoer`"),
        // A boot guest declared on demand under another name is still just
        // a name; the undeclared one refuses, naming it.
        (
            "nonesuch",
            on_demand("other", test_programs::LINK_ECHOER),
            "refused",
            "no guest `nonesuch` is declared",
        ),
    ];
    for (name, entry, variant, needle) in rows {
        let manifest = requester(test_programs::PLUGINS_REFUSED).guest(entry);
        let runtime = boot(manifest, &[name, variant, needle]).await.expect("assembling runtime");
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
    let status =
        run(manifest, &["plugin", "refused", "`test` namespace"]).await.expect("deployment runs");
    assert_eq!(status, ExitStatus::SUCCESS, "the requester's assertions all held");
}

// Loading a name that is already active attests it — a boot guest, the
// requester itself — with the digest boot recorded for its raw bytes.
#[tokio::test]
async fn plugins_attest() {
    let manifest = requester(test_programs::PLUGINS_ATTEST)
        .guest(GuestEntry::new("echoer", test_programs::LINK_ECHOER));
    let status = run(manifest.clone(), &["echoer", "hashed"]).await.expect("deployment runs");
    assert_eq!(status, ExitStatus::SUCCESS, "a boot guest attests with its digest");
    let status = run(manifest, &["requester", "hashed"]).await.expect("deployment runs");
    assert_eq!(status, ExitStatus::SUCCESS, "the requester attests itself");
}

// Admission does not require a linked export: a component exporting no
// `omnia-test:link/ops` loads (it stays reachable through the host
// `Dispatcher`), and a link call to it fails at the call site — the polyfill
// traps the requester, so the failure is observed here, after the load.
#[tokio::test]
async fn plugins_unlinked() {
    let manifest = requester(test_programs::PLUGINS_UNLINKED)
        .guest(on_demand("noseam", test_programs::LINK_FULL));
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
        .guest(on_demand("plugin", test_programs::LINK_ECHOER));
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
        .guest(on_demand("echoer", test_programs::LINK_ECHOER))
        .guest(on_demand("handler", test_programs::LINK_FULL));
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

/// The echoer with one extra custom section: same behavior, new digest.
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
        requester(test_programs::PLUGINS_LOAD).guest(on_demand("plugin", staged.clone()));
    let runtime = boot(manifest, &[]).await.expect("assembling runtime");

    let first = runtime.load("plugin").await.expect("first load");
    runtime.deregister(first.id()).expect("deregistering the loaded plugin");
    assert_eq!(recorded(&runtime, "plugin"), Registered::Absent);

    let changed = changed_echoer(b"reload");
    std::fs::write(&staged, &changed).expect("re-staging");
    let fresh = runtime.load("plugin").await.expect("re-load");
    assert_ne!(fresh.digest(), first.digest(), "the re-load bound fresh bytes");
    assert_eq!(fresh.digest(), Some(Digest::of(&changed)));
    assert_eq!(recorded(&runtime, "plugin"), Registered::Active(fresh.digest()));
    runtime.shutdown();
}

// The declared digest outlives any one registration: once the staged file
// changes, a re-load refuses rather than binding the name to new bytes.
#[tokio::test]
async fn pinned_reload() {
    let scratch = scratch();
    let staged = stage(&scratch, "plugin.wasm", test_programs::LINK_ECHOER);
    let manifest = requester(test_programs::PLUGINS_LOAD)
        .guest(on_demand("plugin", staged.clone()).digest(digest_of(test_programs::LINK_ECHOER)));
    let runtime = boot(manifest, &[]).await.expect("assembling runtime");

    let first = runtime.load("plugin").await.expect("the pinned bytes load");
    runtime.deregister(first.id()).expect("deregistering the loaded plugin");
    std::fs::write(&staged, changed_echoer(b"pinned")).expect("re-staging");

    let stale = runtime.load("plugin").await.expect_err("the staged bytes miss the pin");
    assert!(
        matches!(&stale, LoadError::Refused(detail) if detail.contains("not its declared digest")),
        "{stale:?}"
    );
    assert_eq!(recorded(&runtime, "plugin"), Registered::Absent, "nothing was admitted");
    runtime.shutdown();
}

// An identity the embedder re-registered outside the load path
// (`Runtime::register` records no digest) is active, so a load attests it as
// it stands — with no digest — rather than re-binding or re-admitting it.
#[tokio::test]
async fn reregister_attests() {
    let manifest = requester(test_programs::PLUGINS_LOAD)
        .guest(on_demand("plugin", test_programs::LINK_ECHOER));
    let runtime = boot(manifest, &[]).await.expect("assembling runtime");

    let first = runtime.load("plugin").await.expect("first load");
    assert!(first.digest().is_some());
    runtime.deregister(first.id()).expect("deregistering the loaded plugin");
    runtime
        .register("plugin", GuestArtifact::bytes(changed_echoer(b"reregister")))
        .await
        .expect("re-registering externally");

    let attested = runtime.load("plugin").await.expect("an active identity attests");
    assert_eq!(attested.id(), first.id());
    assert_eq!(attested.digest(), None, "the embedder's registration hashed nothing");
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
// data this crate consumes: an `on_demand` guest with a `digest` and a
// `package` source, and the `registries` contents that route it. The macro's
// snapshot suite pins the expansion shape; this pins the types and the
// carried data.
mod loader_grammar {
    omnia::runtime!({
        guests: [
            {
                name: "tool",
                package: "acme:tool@1.2.3",
                on_demand: true,
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
    assert_eq!(tool.name, "tool");
    assert!(tool.on_demand);
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
            &'a self, package: &'a str,
        ) -> omnia::futures::future::BoxFuture<'a, Result<Vec<u8>, LoadError>> {
            Box::pin(async move {
                assert_eq!(package, ECHOER_PACKAGE);
                Ok(self.0.clone())
            })
        }
    }

    let bytes = std::fs::read(test_programs::LINK_ECHOER).expect("reading the echoer");
    let manifest = requester(test_programs::PLUGINS_LOAD).guest(echoer_package());
    let mut deployment = deployment(manifest, &["plugin"]).await.expect("building deployment");
    deployment.registry_source(Fixed(bytes));
    let runtime =
        deployment.assemble(Backends::defaults().await).await.expect("assembling runtime");
    let status = runtime.run_command().await.expect("deployment runs");
    assert_eq!(status, ExitStatus::SUCCESS, "the requester's assertions all held");
    runtime.shutdown();
}
