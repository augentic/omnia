//! `omnia::runtime!` wiring, manifest lowering, and compile-fail diagnostics.

#![cfg(not(target_arch = "wasm32"))]

use std::future::{Future, ready};
use std::path::PathBuf;
use std::sync::atomic::Ordering;

use omnia::wasmtime::component::Linker;
use omnia::{
    DeploymentBuilder, ExitStatus, Host, HostCtx, Manifest, ManifestSource, Mount, PluginsConfig,
    Provides, Runtime, SourceSpec, StoreCtx,
};

trait TagClient: Send + Sync + 'static {
    fn id(&self) -> usize;
}

#[derive(HostCtx)]
#[host_ctx(client = TagClient, crate = omnia)]
struct WasiAlpha;

impl<B: Provides<Self>> Host<B> for WasiAlpha {
    fn add_to_linker(_linker: &mut Linker<StoreCtx<B>>) -> omnia::anyhow::Result<()> {
        Ok(())
    }

    fn run(runtime: &Runtime<B>) -> impl Future<Output = omnia::anyhow::Result<()>> + Send {
        let mut store = runtime.build_store(runtime.store());
        assert_eq!(store.data_mut().view::<Self>().client.id(), 1);
        ready(Ok(()))
    }
}

#[derive(HostCtx)]
#[host_ctx(client = TagClient, crate = omnia)]
struct WasiBeta;

impl<B: Provides<Self>> Host<B> for WasiBeta {
    fn add_to_linker(_linker: &mut Linker<StoreCtx<B>>) -> omnia::anyhow::Result<()> {
        Ok(())
    }

    fn run(runtime: &Runtime<B>) -> impl Future<Output = omnia::anyhow::Result<()>> + Send {
        let mut store = runtime.build_store(runtime.store());
        assert_eq!(store.data_mut().view::<Self>().client.id(), 1);
        ready(Ok(()))
    }
}

#[derive(HostCtx)]
#[host_ctx(client = TagClient, crate = omnia)]
struct WasiGamma;

impl<B: Provides<Self>> Host<B> for WasiGamma {
    fn add_to_linker(_linker: &mut Linker<StoreCtx<B>>) -> omnia::anyhow::Result<()> {
        Ok(())
    }

    fn run(runtime: &Runtime<B>) -> impl Future<Output = omnia::anyhow::Result<()>> + Send {
        let mut store = runtime.build_store(runtime.store());
        assert_eq!(store.data_mut().view::<Self>().client.id(), 1);
        ready(Ok(()))
    }
}

#[derive(Clone, Copy)]
struct StubClient;

impl TagClient for StubClient {
    fn id(&self) -> usize {
        1
    }
}

#[derive(Clone, Copy)]
struct StubBundle(StubClient);

impl Provides<WasiAlpha> for StubBundle {
    fn borrow(&mut self) -> &dyn TagClient {
        &self.0
    }
}

impl Provides<WasiBeta> for StubBundle {
    fn borrow(&mut self) -> &dyn TagClient {
        &self.0
    }
}

impl Provides<WasiGamma> for StubBundle {
    fn borrow(&mut self) -> &dyn TagClient {
        &self.0
    }
}

mod connect_options {
    use std::future::{Future, ready};
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::{TagClient, WasiAlpha, WasiBeta, WasiGamma};

    pub static OPT_CONNECTS: AtomicUsize = AtomicUsize::new(0);
    pub static OPT_ARG: Mutex<Option<&'static str>> = Mutex::new(None);
    pub static PLAIN_CONNECTS: AtomicUsize = AtomicUsize::new(0);

    #[derive(Clone, Copy)]
    struct Opts(&'static str);

    impl omnia::FromEnv for Opts {
        fn load_env() -> omnia::anyhow::Result<Self> {
            Ok(Self("env"))
        }
    }

    #[derive(Clone)]
    struct OptBackend(usize);

    impl TagClient for OptBackend {
        fn id(&self) -> usize {
            self.0
        }
    }

    impl omnia::Connect for OptBackend {
        type ConnectOptions = Opts;

        fn connect_with(
            options: Self::ConnectOptions,
        ) -> impl Future<Output = omnia::anyhow::Result<Self>> {
            let id = OPT_CONNECTS.fetch_add(1, Ordering::SeqCst) + 1;
            *OPT_ARG.lock().expect("mutex") = Some(options.0);
            ready(Ok(Self(id)))
        }
    }

    #[derive(Clone)]
    struct PlainBackend(usize);

    impl TagClient for PlainBackend {
        fn id(&self) -> usize {
            self.0
        }
    }

    impl omnia::Connect for PlainBackend {
        type ConnectOptions = omnia::NoOptions;

        fn connect_with(
            _options: Self::ConnectOptions,
        ) -> impl Future<Output = omnia::anyhow::Result<Self>> {
            let id = PLAIN_CONNECTS.fetch_add(1, Ordering::SeqCst) + 1;
            ready(Ok(Self(id)))
        }
    }

    omnia::runtime!({
        hosts: {
            WasiAlpha: OptBackend(Opts("custom")),
            WasiBeta: PlainBackend,
            WasiGamma: OptBackend(Opts("custom")),
        },
    });
}

mod shared_backend {
    use std::future::{Future, ready};
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::{TagClient, WasiAlpha, WasiBeta, WasiGamma};

    pub static SHARED_CONNECTS: AtomicUsize = AtomicUsize::new(0);
    pub static PLAIN_CONNECTS: AtomicUsize = AtomicUsize::new(0);

    #[derive(Clone)]
    struct SharedPlain(usize);

    impl TagClient for SharedPlain {
        fn id(&self) -> usize {
            self.0
        }
    }

    impl omnia::Connect for SharedPlain {
        type ConnectOptions = omnia::NoOptions;

        fn connect_with(
            _options: Self::ConnectOptions,
        ) -> impl Future<Output = omnia::anyhow::Result<Self>> {
            let id = SHARED_CONNECTS.fetch_add(1, Ordering::SeqCst) + 1;
            ready(Ok(Self(id)))
        }
    }

    #[derive(Clone)]
    struct PlainBackend(usize);

    impl TagClient for PlainBackend {
        fn id(&self) -> usize {
            self.0
        }
    }

    impl omnia::Connect for PlainBackend {
        type ConnectOptions = omnia::NoOptions;

        fn connect_with(
            _options: Self::ConnectOptions,
        ) -> impl Future<Output = omnia::anyhow::Result<Self>> {
            let id = PLAIN_CONNECTS.fetch_add(1, Ordering::SeqCst) + 1;
            ready(Ok(Self(id)))
        }
    }

    omnia::runtime!({
        hosts: {
            WasiAlpha: SharedPlain,
            WasiBeta: PlainBackend,
            WasiGamma: SharedPlain,
        },
    });
}

mod manifest_file {
    omnia::runtime!({
        manifest: concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.toml"),
    });
}

mod embedded_guests {
    omnia::runtime!({
        mode: command,
        guests: [
            { path: "../Cargo.toml", command: true },
            { path: concat!(env!("CARGO_MANIFEST_DIR"), "/README.md") },
        ],
    });
}

mod deployment_keys {
    use std::path::PathBuf;

    fn project_root() -> PathBuf {
        PathBuf::from("/workspace/project")
    }

    fn store_root() -> PathBuf {
        PathBuf::from("/workspace/store")
    }

    omnia::runtime!({
        mode: command,
        guests: [
            { name: "specify", path: "../Cargo.toml", command: true },
            { name: "target:mock", path: "runtime.rs" },
        ],
        mounts: [
            { name: "project", path: project_root(), writable: true },
            { name: "store", path: store_root() },
        ],
        plugins: {
            store: "store",
        },
    });
}

// Non-adjacent rows sharing `Backend(options)` share one `connect_with` call.
#[test]
fn connect_options() {
    let _ = (connect_options::main, connect_options::run_with::<StubBundle>);

    let empty = connect_options::manifest().into_manifest().expect("default inline manifest");
    assert!(empty.guests.is_empty());

    let guest = Manifest::from_wasm(test_programs::LINK_ECHOER).expect("reading echoer");
    let status =
        connect_options::run(DeploymentBuilder::new().manifest(guest)).expect("deployment runs");
    assert_eq!(status, ExitStatus::SUCCESS);
    assert_eq!(connect_options::OPT_CONNECTS.load(Ordering::SeqCst), 1);
    assert_eq!(*connect_options::OPT_ARG.lock().expect("mutex"), Some("custom"));
    assert_eq!(connect_options::PLAIN_CONNECTS.load(Ordering::SeqCst), 1);
}

// Interleaved rows sharing a bare backend connect once; `run_with` connects nothing.
#[test]
fn shared_backend() {
    let _ = (shared_backend::main, shared_backend::manifest);

    let guest = Manifest::from_wasm(test_programs::LINK_ECHOER).expect("reading echoer");
    let status =
        shared_backend::run(DeploymentBuilder::new().manifest(guest)).expect("deployment runs");
    assert_eq!(status, ExitStatus::SUCCESS);
    assert_eq!(shared_backend::SHARED_CONNECTS.load(Ordering::SeqCst), 1);
    assert_eq!(shared_backend::PLAIN_CONNECTS.load(Ordering::SeqCst), 1);

    // `run_with` drives the same host wiring over a caller-supplied bundle
    let guest = Manifest::from_wasm(test_programs::LINK_ECHOER).expect("reading echoer");
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().expect("tokio");
    let status = rt
        .block_on(shared_backend::run_with(
            DeploymentBuilder::new().manifest(guest),
            StubBundle(StubClient),
        ))
        .expect("run_with succeeds");
    assert_eq!(status, ExitStatus::SUCCESS);
    assert_eq!(shared_backend::SHARED_CONNECTS.load(Ordering::SeqCst), 1);
}

#[test]
fn manifest_file() {
    let _ = (manifest_file::main, manifest_file::run, manifest_file::run_with::<()>);
    assert!(
        matches!(manifest_file::manifest(), ManifestSource::Path(path) if path.ends_with("Cargo.toml"))
    );
}

#[test]
fn inline_deployment() {
    let _ = (embedded_guests::main, embedded_guests::run, embedded_guests::run_with::<()>);
    let _ = (deployment_keys::main, deployment_keys::run, deployment_keys::run_with::<()>);

    // stem-derived names, `command: true` vs default `false`, and macro-valued `path:`
    let embedded = embedded_guests::manifest().into_manifest().expect("inline manifest");
    let [cargo, readme] = embedded.guests.as_slice() else {
        panic!("two embedded guests: {:?}", embedded.guests);
    };
    assert_eq!(cargo.name, "Cargo");
    assert!(cargo.command);
    assert_eq!(cargo.source, SourceSpec::from(include_bytes!("../Cargo.toml").as_slice()));

    assert_eq!(readme.name, "README");
    assert!(!readme.command);
    assert_eq!(
        readme.source,
        SourceSpec::from(
            include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/README.md")).as_slice()
        )
    );

    // explicit guest names, expression-valued mounts, and store-only `plugins`
    let keyed = deployment_keys::manifest().into_manifest().expect("inline manifest");
    let [specify, mock] = keyed.guests.as_slice() else {
        panic!("two named guests: {:?}", keyed.guests);
    };
    assert_eq!(specify.name, "specify");
    assert!(specify.command);
    assert_eq!(specify.source, SourceSpec::from(include_bytes!("../Cargo.toml").as_slice()));

    assert_eq!(mock.name, "target:mock");
    assert!(!mock.command);
    assert_eq!(mock.source, SourceSpec::from(include_bytes!("runtime.rs").as_slice()));

    assert_eq!(
        keyed.mounts,
        [
            Mount {
                name: "project".to_owned(),
                path: PathBuf::from("/workspace/project"),
                writable: true,
            },
            Mount {
                name: "store".to_owned(),
                path: PathBuf::from("/workspace/store"),
                writable: false,
            },
        ]
    );
    assert_eq!(
        keyed.plugins,
        Some(PluginsConfig {
            store: Some(PathBuf::from("store")),
            registries: None,
        })
    );
}

#[test]
fn compile_fail_diagnostics() {
    trybuild::TestCases::new().compile_fail("tests/ui/runtime_invalid.rs");
}
