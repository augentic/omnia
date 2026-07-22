//! Composed WASI parity for the guest command router, driven exactly as a
//! one-shot command deployment would.
//!
//! One table-driven test covers every operation route plus router-generated
//! help, version, and usage behavior, including arbitrary nonzero codes
//! carried by p3 `wasi:cli/exit`. Each case runs the full public `run()` path
//! (deployment build, command routing, exit mapping); the serialized guest
//! artifact keeps the per-case build cheap.
//!
//! The `command_guest_*` tests cover the explicit command guest: an empty
//! dynamic registry resolving the command guest on the first miss, resolver
//! absence/failure, wrong-export refusal, `argv[0]` via `program_name`, and
//! exit-code passthrough — plus static-deployment compatibility.

use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use anyhow::{Context as _, Result, ensure};
use futures::FutureExt as _;
use omnia::{
    Deployment, DeploymentBuilder, ExitStatus, FutureResult, GuestArtifact, GuestId, GuestResolver,
    Manifest, Mode, Runtime, StoreCtx, Wiring, run, run_precompiled,
};
use omnia_testkit::find_guest;

use crate::fixture;

/// Serialize the module's tests: each run builds a fresh engine whose pooling
/// allocator reserves a large virtual mapping, and too many alive at once
/// exhaust the address space the OS grants the suite process.
static ENGINE_GATE: Mutex<()> = Mutex::new(());

fn engine_gate() -> MutexGuard<'static, ()> {
    ENGINE_GATE.lock().unwrap_or_else(PoisonError::into_inner)
}

struct EmptyWiring;

impl Wiring<()> for EmptyWiring {
    fn link(_deployment: &mut Deployment<StoreCtx<()>>) -> Result<()> {
        Ok(())
    }

    async fn serve(_runtime: &Runtime<()>) -> Result<()> {
        Ok(())
    }
}

/// Drive `wasi:cli/run` once with `tail` guest argv (the program name is
/// prepended by command mode) and return the guest's exit status.
async fn run_cli(wasm: &Path, tail: &[&str]) -> Result<ExitStatus> {
    // The `()` bundle links no hosts; `wasi:cli` is wired by the deployment
    // builder, and `Runtime::new` threads the guest argv into every store.
    let builder = DeploymentBuilder::new()
        .manifest(Manifest::from_wasm(wasm))
        .args(tail.iter().map(|arg| (*arg).to_string()).collect::<Vec<_>>())
        .mode(Mode::Command)
        .precompiled();
    // SAFETY: `find_guest` only returns artifacts this workspace built and
    // serialized itself (`cargo make test-guests`).
    unsafe { run_precompiled::<(), EmptyWiring>(builder) }.await.context("running command")
}

#[test]
fn exit_codes() -> Result<()> {
    let _gate = engine_gate();
    fixture::RT.block_on(async {
        let wasm = find_guest("cli_wasm.wasm");

        let cases: &[(&[&str], i32, &str)] = &[
            (&["greet", "Ada"], 0, "greet exits 0"),
            (&["greet"], 0, "default greeting exits 0"),
            (&["add", "2", "40"], 0, "add exits 0"),
            (&["env"], 0, "env exits 0"),
            (&["--help"], 0, "clap-generated --help exits 0"),
            (&["--version"], 0, "clap-generated --version exits 0"),
            (&["bogus"], 2, "clap usage error exits 2"),
            (&[], 2, "clap usage error exits 2"),
            (&["fail", "42"], 42, "wasi:cli/exit carries a specific code"),
            (&["fail"], 1, "Err(()) from run maps to 1"),
        ];

        for (tail, code, expectation) in cases {
            let status = run_cli(&wasm, tail).await?;
            assert_eq!(status.code(), *code, "{expectation} (argv: {tail:?})");
        }

        Ok(())
    })
}

/// A counting command-guest resolver answering every identity with
/// `answer()`'s outcome.
struct CommandResolver<F> {
    calls: Arc<AtomicUsize>,
    answer: F,
}

impl<F> CommandResolver<F>
where
    F: Fn() -> Result<Option<GuestArtifact>> + Send + Sync + 'static,
{
    fn new(answer: F) -> (Arc<Self>, Arc<AtomicUsize>) {
        let calls = Arc::new(AtomicUsize::new(0));
        (
            Arc::new(Self {
                calls: Arc::clone(&calls),
                answer,
            }),
            calls,
        )
    }
}

impl<F> GuestResolver for CommandResolver<F>
where
    F: Fn() -> Result<Option<GuestArtifact>> + Send + Sync + 'static,
{
    fn resolve(
        &self, _guest: GuestId, _expected_export: String,
    ) -> FutureResult<Option<GuestArtifact>> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let outcome = (self.answer)();
        async move { outcome }.boxed()
    }
}

/// The serialized CLI guest wrapped as a registration artifact.
fn cli_artifact() -> Result<GuestArtifact> {
    let path = find_guest("cli_wasm.wasm");
    ensure!(
        path.extension().is_some_and(|ext| ext == "bin"),
        "{} has no serialized .bin sibling; run `cargo make test-guests`",
        path.display()
    );
    let bytes =
        std::fs::read(&path).with_context(|| format!("reading guest {}", path.display()))?;
    // SAFETY: the artifact was built and serialized by this workspace's own
    // `cargo make test-guests` pipeline (omnia's compile path).
    Ok(unsafe { GuestArtifact::precompiled(bytes) })
}

/// Drive an empty dynamic deployment whose explicit command guest resolves
/// through `resolver`, returning the run outcome.
async fn run_dynamic_cli(
    resolver: Option<Arc<dyn GuestResolver>>, tail: &[&str],
) -> Result<ExitStatus> {
    let mut builder = DeploymentBuilder::new()
        .dynamic()
        .command_guest("app")
        .program_name("app")
        .args(tail.iter().map(|arg| (*arg).to_string()).collect::<Vec<_>>())
        .mode(Mode::Command);
    if let Some(resolver) = resolver {
        builder = builder.resolver(resolver);
    }
    run::<(), EmptyWiring>(builder).await
}

// An empty dynamic registry resolves the explicit command guest on the first
// (and only) miss; the guest runs and its exit codes pass through unchanged.
#[test]
fn command_guest_resolved() -> Result<()> {
    let _gate = engine_gate();
    fixture::RT.block_on(async {
        let cases: &[(&[&str], i32, &str)] = &[
            (&["greet", "Ada"], 0, "greet exits 0"),
            (&["fail", "42"], 42, "wasi:cli/exit carries a specific code"),
            (&["fail"], 1, "Err(()) from run maps to 1"),
        ];

        for (tail, code, expectation) in cases {
            let (resolver, calls) = CommandResolver::new(|| Ok(Some(cli_artifact()?)));
            let status = run_dynamic_cli(Some(resolver), tail).await?;
            assert_eq!(status.code(), *code, "{expectation} (argv: {tail:?})");
            assert_eq!(calls.load(Ordering::SeqCst), 1, "one miss, one resolution");
        }

        Ok(())
    })
}

// An explicit command guest with no resolver installed fails the run — never
// the previous inert exit 0.
#[test]
fn command_guest_unresolved_fails() -> Result<()> {
    let _gate = engine_gate();
    fixture::RT.block_on(async {
        let error = run_dynamic_cli(None, &["greet"])
            .await
            .expect_err("an unresolvable command guest must fail the run");
        assert!(format!("{error:#}").contains("is not registered"), "{error:#}");
        Ok(())
    })
}

// A resolver decline (`Ok(None)`) is a definitive miss: the run fails.
#[test]
fn command_guest_declined_fails() -> Result<()> {
    let _gate = engine_gate();
    fixture::RT.block_on(async {
        let (resolver, _calls) = CommandResolver::new(|| Ok(None));
        let error = run_dynamic_cli(Some(resolver), &["greet"])
            .await
            .expect_err("a declined command guest must fail the run");
        assert!(format!("{error:#}").contains("is not registered"), "{error:#}");
        Ok(())
    })
}

// A resolver failure surfaces with its cause chain intact — the seam an
// embedder downcasts through for typed error rendering.
#[test]
fn command_guest_resolver_failure_surfaces() -> Result<()> {
    let _gate = engine_gate();
    fixture::RT.block_on(async {
        let (resolver, _calls) = CommandResolver::new(|| Err(anyhow::anyhow!("store outage")));
        let error = run_dynamic_cli(Some(resolver), &["greet"])
            .await
            .expect_err("a failing resolver must fail the run");
        let chain = format!("{error:#}");
        assert!(chain.contains("guest resolution failed"), "{chain}");
        assert!(chain.contains("store outage"), "the cause chain is preserved: {chain}");
        Ok(())
    })
}

// A resolved component that does not export `wasi:cli/run` is refused and
// leaves no partial state.
#[test]
fn command_guest_wrong_export_refused() -> Result<()> {
    let _gate = engine_gate();
    fixture::RT.block_on(async {
        // The link responder exports `omnia:link/echo`, not `wasi:cli/run`.
        let (resolver, _calls) = CommandResolver::new(|| {
            let path = find_guest("guest_link_responder_wasm.wasm");
            let bytes = std::fs::read(&path)
                .with_context(|| format!("reading guest {}", path.display()))?;
            // SAFETY: workspace-built artifact (`cargo make test-guests`).
            Ok(Some(unsafe { GuestArtifact::precompiled(bytes) }))
        });
        let error = run_dynamic_cli(Some(resolver), &["greet"])
            .await
            .expect_err("a wrong-export command guest must fail the run");
        assert!(
            format!("{error:#}").contains("does not export interface `wasi:cli/run`"),
            "{error:#}"
        );
        Ok(())
    })
}

// `program_name` overrides `argv[0]` (command mode prepends the deployment
// name); the manifest-derived default is unchanged without it.
#[test]
fn command_guest_program_name_sets_argv0() -> Result<()> {
    let _gate = engine_gate();
    fixture::RT.block_on(async {
        let deployment = DeploymentBuilder::new()
            .dynamic()
            .command_guest("app")
            .program_name("myprog")
            .args(vec!["greet".to_owned()])
            .mode(Mode::Command)
            .build::<StoreCtx<()>>()
            .await?;
        assert_eq!(deployment.args(), ["myprog", "greet"], "program_name overrides argv[0]");

        let deployment = DeploymentBuilder::new()
            .dynamic()
            .args(vec!["greet".to_owned()])
            .mode(Mode::Command)
            .build::<StoreCtx<()>>()
            .await?;
        assert_eq!(deployment.args(), ["omnia", "greet"], "the manifest default is unchanged");

        Ok(())
    })
}

// An explicit command guest naming a static `[[guest]]` entry is a registry
// hit — no resolver consulted, ordinary static deployments keep working.
#[test]
fn command_guest_static_hit() -> Result<()> {
    let _gate = engine_gate();
    fixture::RT.block_on(async {
        let wasm = find_guest("cli_wasm.wasm");
        let (resolver, calls) = CommandResolver::new(|| Ok(Some(cli_artifact()?)));
        let builder = DeploymentBuilder::new()
            .manifest(Manifest::from_wasm(&wasm))
            .command_guest("cli_wasm")
            .args(vec!["add".to_owned(), "2".to_owned(), "40".to_owned()])
            .mode(Mode::Command)
            .resolver(resolver)
            .precompiled();
        // SAFETY: `find_guest` only returns artifacts this workspace built and
        // serialized itself (`cargo make test-guests`).
        let status = unsafe { run_precompiled::<(), EmptyWiring>(builder) }.await?;
        assert_eq!(status.code(), 0, "the static command guest runs");
        assert_eq!(calls.load(Ordering::SeqCst), 0, "a registry hit never resolves");
        Ok(())
    })
}
