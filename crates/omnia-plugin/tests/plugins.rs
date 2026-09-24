//! End-to-end tests for the `omnia:plugins/loader` host capability: a real
//! requester guest from `crates/test-programs` drives loads through omnia's
//! runtime, with `PathMounts` reading components staged in a scratch mount or
//! a `RegistryClient` resolving them from a wasm-pkg `local` backend. The
//! requester asserts internally (handles, digests, dispatch answers, and
//! every typed refusal); the host side stages artifacts and checks the exit.
//! Lifecycle scenarios the WASI surface cannot reach (deregistration) drive
//! [`PluginLoader`] host-side over the same runtime.

#![cfg(not(target_arch = "wasm32"))]

use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context as _, Result, bail};
use omnia::wasmtime::component::Val;
use omnia::{
    ChainCtx, DeploymentBuilder, ExitStatus, GuestArtifact, GuestEntry, GuestId, LoadError,
    Manifest, Mode, Origin, PathMounts, PathSource, PluginLoader as _, RegistryClient,
    RegistryConfig, RegistrySource, Runtime, StoreCtx, sha256_digest,
};
use omnia_test::host::{Backends, Scratch, scratch};
use omnia_wasi_otel::WasiOtel;

// Every guest program in `crates/test-programs/programs/plugins` must have a
// matching test here; a new program without one fails to compile.
test_programs::foreach_plugins!();

/// Build the requester deployment around `wasm`: the scratch dir mounts at
/// `.`, and `args` is the guest's argv past the program name. The telemetry
/// host serves the `command!` guest's otel imports; the loader host is
/// assembly's. Nothing declares what the requester imports: the seam is
/// read off the components.
async fn requester_deployment(
    wasm: &str, scratch: &Scratch, args: &[&str],
) -> Result<omnia::Deployment<StoreCtx<Backends>>> {
    let manifest =
        Manifest::new().guest(GuestEntry::new("requester", wasm)).mounts([scratch.mount(false)]);
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

/// Assemble the requester deployment over a custom acquisition policy: the
/// two slots selected through `Deployment::loader` before assembly.
async fn requester_runtime(
    wasm: &str, scratch: &Scratch, registry: Option<Arc<dyn RegistrySource>>,
    path: Option<Arc<dyn PathSource>>,
) -> Result<Runtime<Backends>> {
    let mut deployment = requester_deployment(wasm, scratch, &[]).await?;
    deployment.loader(registry, path);
    deployment.assemble(Backends::defaults().await).await.context("assembling runtime")
}

/// Assemble the requester deployment over the declared policy — the `.`
/// mount serves path loads — with `args` as the guest's argv.
async fn declared_runtime(
    wasm: &str, scratch: &Scratch, args: &[&str],
) -> Result<Runtime<Backends>> {
    requester_deployment(wasm, scratch, args)
        .await?
        .assemble(Backends::defaults().await)
        .await
        .context("assembling runtime")
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

/// Drive `wasm` as the `requester` command guest under the given slots.
async fn run_requester(
    wasm: &str, scratch: &Scratch, registry: Option<Arc<dyn RegistrySource>>,
    path: Option<Arc<dyn PathSource>>,
) -> Result<ExitStatus> {
    let runtime = requester_runtime(wasm, scratch, registry, path).await?;
    runtime.run_command().await
}

/// Stage `wasm` in the scratch dir under `name`.
fn stage(scratch: &Scratch, name: &str, wasm: &str) {
    let target = scratch.path().join(name);
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent).expect("creating the staging directory");
    }
    std::fs::copy(wasm, target).unwrap_or_else(|error| panic!("staging {name}: {error}"));
}

/// A `.`-rooted `PathMounts` over the scratch dir filling the path slot.
fn path_source(scratch: &Scratch) -> Arc<dyn PathSource> {
    Arc::new(PathMounts::new([(".", scratch.path())]).expect("opening the scratch location"))
}

/// A path load of `./plugin.wasm`, the echoer staged under that name.
fn plugin_origin() -> Origin {
    Origin::Path("./plugin.wasm".to_owned())
}

#[derive(serde::Serialize)]
struct LocalBackendConfig {
    root: PathBuf,
}

/// Stage `wasm` as `package` in a wasm-pkg `local` backend rooted at `root`,
/// served by the registry `registry.test`; `routed` makes that registry the
/// client's default, otherwise a load must name it.
fn registry_config(
    root: &Path, package: &str, wasm: &str, routed: bool,
) -> wasm_pkg_client::Config {
    let (name, version) = package.split_once('@').expect("test packages pin versions");
    let (namespace, name) = name.split_once(':').expect("test packages are namespaced");
    let dir = root.join(namespace).join(name);
    std::fs::create_dir_all(&dir).expect("creating package directory");
    std::fs::copy(wasm, dir.join(format!("{version}.wasm"))).expect("staging package");

    let registry: wasm_pkg_client::Registry =
        "registry.test".parse().expect("test registry name parses");
    let mut config = wasm_pkg_client::Config::empty();
    if routed {
        config.set_default_registry(Some(registry.clone()));
    }
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
    config
}

/// A client whose default registry `registry.test` serves `package`.
fn registry_source(root: &Path, package: &str, wasm: &str) -> Arc<dyn RegistrySource> {
    Arc::new(RegistryClient::new(registry_config(root, package, wasm, true)))
}

// The declared policy: the `.` mount is the root a path load resolves
// against, with no loader source named anywhere.
#[tokio::test]
async fn plugins_load_path() {
    let scratch = scratch();
    stage(&scratch, "plugin.wasm", test_programs::LINK_ECHOER);

    let runtime = declared_runtime(test_programs::PLUGINS_LOAD_PATH, &scratch, &[])
        .await
        .expect("assembling runtime");
    let status = runtime.run_command().await.expect("deployment runs");
    assert_eq!(status, ExitStatus::SUCCESS, "the requester's assertions all held");
    runtime.shutdown();
}

// A custom path policy selected before assembly serves the same load.
#[tokio::test]
async fn custom_path_source() {
    let scratch = scratch();
    stage(&scratch, "plugin.wasm", test_programs::LINK_ECHOER);

    let status = run_requester(
        test_programs::PLUGINS_LOAD_PATH,
        &scratch,
        None,
        Some(path_source(&scratch)),
    )
    .await
    .expect("deployment runs");
    assert_eq!(status, ExitStatus::SUCCESS, "the requester's assertions all held");
}

// A path load registers as its file stem, wherever the file sits under the
// mount, and that stem is the identity the host dispatches by.
#[tokio::test]
async fn path_named_by_stem() {
    let scratch = scratch();
    stage(&scratch, "adapters/tool.wasm", test_programs::LINK_ECHOER);
    let runtime = declared_runtime(test_programs::PLUGINS_LOAD_PATH, &scratch, &[])
        .await
        .expect("assembling runtime");

    let plugin = runtime
        .load(Origin::Path("./adapters/tool.wasm".to_owned()), None)
        .await
        .expect("a nested path loads");
    assert_eq!(plugin.id(), &GuestId::from("tool"));
    let answer = invoke_ping(&runtime, "tool", "hi").await.expect("host dispatch by stem");
    assert_eq!(answer, "tool pong: hi");
    runtime.shutdown();
}

#[tokio::test]
async fn plugins_load_registry() {
    let scratch = scratch();
    let registry = registry_source(scratch.path(), "test:echoer@1.0.0", test_programs::LINK_ECHOER);

    let status =
        run_requester(test_programs::PLUGINS_LOAD_REGISTRY, &scratch, Some(registry), None)
            .await
            .expect("deployment runs");
    assert_eq!(status, ExitStatus::SUCCESS, "the requester's assertions all held");
}

// A load naming its registry reaches it past the deployment's routing: the
// configuration routes nothing, and the named endpoint serves the package.
#[tokio::test]
async fn named_registry() {
    let scratch = scratch();
    let config =
        registry_config(scratch.path(), "test:echoer@1.0.0", test_programs::LINK_ECHOER, false);
    let runtime = requester_runtime(
        test_programs::PLUGINS_LOAD_PATH,
        &scratch,
        Some(Arc::new(RegistryClient::new(config))),
        None,
    )
    .await
    .expect("assembling runtime");

    let unnamed = Origin::Registry {
        package: "test:echoer@1.0.0".to_owned(),
        endpoint: None,
    };
    let error = runtime.load(unnamed, None).await.expect_err("nothing routes the package");
    assert!(
        matches!(&error, LoadError::Refused(detail) if detail.contains("the load names none")),
        "{error:?}"
    );

    let named = Origin::Registry {
        package: "test:echoer@1.0.0".to_owned(),
        endpoint: Some("registry.test".to_owned()),
    };
    let plugin = runtime.load(named, None).await.expect("the named registry serves the package");
    assert_eq!(plugin.id(), &GuestId::from("test:echoer@1.0.0"));
    let answer = invoke_ping(&runtime, "test:echoer@1.0.0", "hi").await.expect("host dispatch");
    assert_eq!(answer, "test:echoer@1.0.0 pong: hi");
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

// Compile-time proof that the macro's inline keys lower into manifest data
// this crate's `Plugins::install_declared` consumes: the mounts fold into a
// `PathMounts`, the `registries` contents into a `RegistryClient`, each
// filling its slot on `Plugins`. The macro's snapshot suite pins the
// expansion shape; this pins the types and the carried data.
mod loader_grammar {
    omnia::runtime!({
        registries: "default_registry = \"ghcr.io\"\n",
        mounts: [{ name: "adapters", path: "adapters" }],
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
    assert_eq!(manifest.mounts[0].name, "adapters");
    assert!(manifest.guests.is_empty(), "the guests arrive at run time");
}

#[tokio::test]
async fn plugins_load_refused() {
    let scratch = scratch();
    stage(&scratch, "plugin.wasm", test_programs::LINK_ECHOER);
    // A path whose stem is the deployment's own `requester`.
    stage(&scratch, "requester.wasm", test_programs::LINK_ECHOER);
    // Leading ELF magic is exactly what the loader sniffs; the tail is junk,
    // proving refusal happens before any wasmtime parsing.
    std::fs::write(scratch.path().join("native.bin"), [0x7f, b'E', b'L', b'F', 0, 0, 0, 0])
        .expect("staging native bytes");

    let status = run_requester(
        test_programs::PLUGINS_LOAD_REFUSED,
        &scratch,
        None,
        Some(path_source(&scratch)),
    )
    .await
    .expect("deployment runs");
    assert_eq!(status, ExitStatus::SUCCESS, "the requester's assertions all held");
}

// A declared name attests the deployment's own guest through the WIT: the
// handle carries the name and no digest, and nothing is read.
#[tokio::test]
async fn plugins_load_declared() {
    let scratch = scratch();
    let runtime =
        declared_runtime(test_programs::PLUGINS_LOAD_DECLARED, &scratch, &["requester", "attests"])
            .await
            .expect("assembling runtime");
    let status = runtime.run_command().await.expect("deployment runs");
    assert_eq!(status, ExitStatus::SUCCESS, "the requester's assertions all held");
    runtime.shutdown();
}

// A name the deployment does not declare refuses typed, naming the guest.
#[tokio::test]
async fn declared_refused() {
    let scratch = scratch();
    let runtime = declared_runtime(
        test_programs::PLUGINS_LOAD_DECLARED,
        &scratch,
        &["nonesuch", "undeclared"],
    )
    .await
    .expect("assembling runtime");
    let status = runtime.run_command().await.expect("deployment runs");
    assert_eq!(status, ExitStatus::SUCCESS, "the requester's assertions all held");
    runtime.shutdown();
}

// A declared name takes no pin: there are no bytes to hold it to.
#[tokio::test]
async fn declared_takes_no_pin() {
    let scratch = scratch();
    let runtime =
        declared_runtime(test_programs::PLUGINS_LOAD_DECLARED, &scratch, &["requester", "pinned"])
            .await
            .expect("assembling runtime");
    let status = runtime.run_command().await.expect("deployment runs");
    assert_eq!(status, ExitStatus::SUCCESS, "the requester's assertions all held");
    runtime.shutdown();
}

// A declared load of a guest an earlier load admitted attests it with the
// digest that load recorded.
#[tokio::test]
async fn declared_attests() {
    let scratch = scratch();
    stage(&scratch, "plugin.wasm", test_programs::LINK_ECHOER);
    let runtime = declared_runtime(test_programs::PLUGINS_LOAD_PATH, &scratch, &[])
        .await
        .expect("assembling runtime");

    let loaded = runtime.load(plugin_origin(), None).await.expect("the path load");
    let declared = runtime
        .load(Origin::Declared("plugin".to_owned()), None)
        .await
        .expect("the loaded guest is declared now");
    assert_eq!(declared.id(), loaded.id());
    assert_eq!(declared.digest(), loaded.digest());
    assert!(declared.digest().is_some(), "the recorded digest travels with the attestation");

    let static_guest = runtime
        .load(Origin::Declared("requester".to_owned()), None)
        .await
        .expect("a deployment guest is declared");
    assert_eq!(static_guest.id(), &GuestId::from("requester"));
    assert_eq!(static_guest.digest(), None, "assembly hashed nothing");
    runtime.shutdown();
}

// Admission does not require a linked export: a component exporting no
// `omnia-test:link/ops` loads (it stays reachable through the host
// `Dispatcher`), and a link call to it fails at the call site — the polyfill
// traps the requester, so the failure is observed here, after the load.
#[tokio::test]
async fn plugins_load_unlinked() {
    let scratch = scratch();
    stage(&scratch, "noseam.wasm", test_programs::LINK_FULL);

    let runtime = requester_runtime(
        test_programs::PLUGINS_LOAD_UNLINKED,
        &scratch,
        None,
        Some(path_source(&scratch)),
    )
    .await
    .expect("assembling runtime");

    let error = runtime.run_command().await.expect_err("the link call traps the requester");
    assert!(
        runtime.registry().get(&"noseam".into()).is_some(),
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
async fn plugins_load_host_only() {
    let scratch = scratch();
    stage(&scratch, "plugin.wasm", test_programs::LINK_ECHOER);

    let runtime = requester_runtime(
        test_programs::PLUGINS_LOAD_HOST_ONLY,
        &scratch,
        None,
        Some(path_source(&scratch)),
    )
    .await
    .expect("assembling runtime");

    let status = runtime.run_command().await.expect("deployment runs");
    assert_eq!(status, ExitStatus::SUCCESS, "the requester's load held");
    assert!(
        runtime.registry().get(&GuestId::from("plugin")).is_some(),
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
async fn plugins_load_mixed() {
    let scratch = scratch();
    stage(&scratch, "echoer.wasm", test_programs::LINK_ECHOER);
    stage(&scratch, "handler.wasm", test_programs::LINK_FULL);

    let runtime = requester_runtime(
        test_programs::PLUGINS_LOAD_MIXED,
        &scratch,
        None,
        Some(path_source(&scratch)),
    )
    .await
    .expect("assembling runtime");

    let status = runtime.run_command().await.expect("deployment runs");
    assert_eq!(status, ExitStatus::SUCCESS, "the requester's assertions all held");
    assert!(
        runtime.registry().get(&GuestId::from("echoer")).is_some(),
        "the link target is registered"
    );
    assert!(
        runtime.registry().get(&GuestId::from("handler")).is_some(),
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

// Host-side loads over the same runtime the guests drive: deregistration is
// host authority, so the WASI surface cannot reach this scenario. A
// deregistered guest's digest record must not survive into the next load —
// the re-load binds the freshly staged bytes, and a stale pin refuses.
#[tokio::test]
async fn reload_after_deregister() {
    let scratch = scratch();
    stage(&scratch, "plugin.wasm", test_programs::LINK_ECHOER);
    // The requester guest is manifest ballast: these loads are host-driven.
    let runtime = requester_runtime(
        test_programs::PLUGINS_LOAD_PATH,
        &scratch,
        None,
        Some(path_source(&scratch)),
    )
    .await
    .expect("assembling runtime");

    let first = runtime.load(plugin_origin(), None).await.expect("first load");
    runtime.deregister(first.id()).expect("deregistering the loaded plugin");

    // Same component, one extra custom section: same behavior, new digest.
    let mut changed = std::fs::read(test_programs::LINK_ECHOER).expect("reading the echoer");
    changed.extend_from_slice(&custom_section(b"reload"));
    std::fs::write(scratch.path().join("plugin.wasm"), &changed).expect("re-staging");

    let stale = runtime
        .load(plugin_origin(), first.digest())
        .await
        .expect_err("the old digest no longer matches the staged bytes");
    match &stale {
        LoadError::Refused(detail) => {
            assert!(detail.contains("does not match the pinned"), "{detail}");
        }
        other => panic!("expected a digest-mismatch refusal: {other:?}"),
    }

    let fresh = runtime.load(plugin_origin(), None).await.expect("re-load");
    assert_ne!(fresh.digest(), first.digest(), "the re-load bound fresh bytes");
    assert_eq!(fresh.digest(), Some(sha256_digest(&changed).as_str()));
    runtime.shutdown();
}

// The digest record lives on the registry entry, so an embedder swapping the
// identity outside the load path (deregister + `Runtime::register`) leaves no
// stale attestation behind: a pinned re-load must refuse rather than answer
// with the old digest over the new bytes.
#[tokio::test]
async fn pinned_reload_reregister() {
    let scratch = scratch();
    stage(&scratch, "plugin.wasm", test_programs::LINK_ECHOER);
    let runtime = requester_runtime(
        test_programs::PLUGINS_LOAD_PATH,
        &scratch,
        None,
        Some(path_source(&scratch)),
    )
    .await
    .expect("assembling runtime");

    let first = runtime.load(plugin_origin(), None).await.expect("first load");
    runtime.deregister(first.id()).expect("deregistering the loaded plugin");

    // Same component, one extra custom section: same behavior, new digest —
    // registered by the embedder, not through the loader.
    let mut changed = std::fs::read(test_programs::LINK_ECHOER).expect("reading the echoer");
    changed.extend_from_slice(&custom_section(b"reregister"));
    runtime
        .register("plugin", GuestArtifact::wasm(changed))
        .await
        .expect("re-registering externally");

    let stale = runtime
        .load(plugin_origin(), first.digest())
        .await
        .expect_err("a load never attests an externally registered guest");
    assert!(matches!(stale, LoadError::AlreadyActive(_)), "{stale:?}");
    runtime.shutdown();
}

// A deployment declaring no mounts and no `registries` still installs the
// loader: a path load refuses typed, naming what the deployment would need,
// and a package load that names no registry finds nothing routing it.
#[tokio::test]
async fn empty_declared_policy() {
    let manifest =
        Manifest::new().guest(GuestEntry::new("requester", test_programs::PLUGINS_LOAD_PATH));
    let mut deployment = DeploymentBuilder::new()
        .manifest(manifest)
        .mode(Mode::Command)
        .build::<StoreCtx<Backends>>()
        .await
        .expect("building deployment");
    deployment.host::<WasiOtel, Backends>().expect("linking the otel host");
    let runtime =
        deployment.assemble(Backends::defaults().await).await.expect("assembling runtime");

    let error = runtime
        .load(plugin_origin(), None)
        .await
        .expect_err("a deployment without mounts refuses every path load");
    assert!(
        matches!(&error, LoadError::Refused(detail) if detail.contains("mounts no directories")),
        "{error:?}"
    );

    let unnamed = Origin::Registry {
        package: "test:echoer@1.0.0".to_owned(),
        endpoint: None,
    };
    let error = runtime
        .load(unnamed, None)
        .await
        .expect_err("a deployment without `registries` routes no unnamed package load");
    assert!(
        matches!(&error, LoadError::Refused(detail) if detail.contains("no registry routes `test:echoer`") && detail.contains("the load names none")),
        "{error:?}"
    );
    runtime.shutdown();
}

// A `registries` configuration that routes nothing for a package's namespace
// refuses the load, naming the namespace, rather than reaching a fallback.
#[tokio::test]
async fn unrouted_package() {
    let scratch = scratch();
    let manifest = Manifest::new()
        .guest(GuestEntry::new("requester", test_programs::PLUGINS_LOAD_PATH))
        .registries(RegistryConfig::contents("[namespace_registries]\nwasi = \"wasi.dev\"\n"))
        .mounts([scratch.mount(false)]);
    let mut deployment = DeploymentBuilder::new()
        .manifest(manifest)
        .mode(Mode::Command)
        .build::<StoreCtx<Backends>>()
        .await
        .expect("building deployment");
    deployment.host::<WasiOtel, Backends>().expect("linking the otel host");
    let runtime =
        deployment.assemble(Backends::defaults().await).await.expect("assembling runtime");

    let unnamed = Origin::Registry {
        package: "test:echoer@1.0.0".to_owned(),
        endpoint: None,
    };
    let error = runtime.load(unnamed, None).await.expect_err("an unrouted namespace refuses");
    assert!(
        matches!(&error, LoadError::Refused(detail) if detail.contains("no registry routes `test:echoer`") && detail.contains("`test` namespace")),
        "{error:?}"
    );
    runtime.shutdown();
}
