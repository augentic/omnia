//! Composed WASI parity for the guest command router, driven exactly as a
//! one-shot command deployment would.
//!
//! One table-driven test covers every operation route plus router-generated
//! help, version, and usage behavior, including arbitrary nonzero codes
//! carried by p3 `wasi:cli/exit`. Each case runs the full public `run()` path
//! (deployment build, command routing, exit mapping); the serialized guest
//! artifact keeps the per-case build cheap.
//!
//! The `command_flag` module covers the `command = true` guest mark: routing
//! to the marked entry (including past a second `wasi:cli/run` exporter that
//! would otherwise be ambiguous), `argv[0]` via `program_name`, and that a
//! registry hit never consults the resolver.

use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use anyhow::{Context as _, Result};
use futures::FutureExt as _;
use omnia::{
    Deployment, DeploymentBuilder, ExitStatus, FutureResult, GuestArtifact, GuestEntry, GuestId,
    GuestResolver, Manifest, Mode, Runtime, StoreCtx, Wiring, run_precompiled,
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

mod command_flag {
    use super::*;

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
        omnia_testkit::precompiled_artifact("cli_wasm.wasm")
    }

    /// Drive a command deployment built from `manifest`, returning the run
    /// outcome.
    async fn run_manifest(manifest: Manifest, tail: &[&str]) -> Result<ExitStatus> {
        let builder = DeploymentBuilder::new()
            .manifest(manifest)
            .args(tail.iter().map(|arg| (*arg).to_string()).collect::<Vec<_>>())
            .mode(Mode::Command)
            .precompiled();
        // SAFETY: `find_guest` only returns artifacts this workspace built and
        // serialized itself (`cargo make test-guests`).
        unsafe { run_precompiled::<(), EmptyWiring>(builder) }.await
    }

    // `program_name` overrides `argv[0]` (command mode prepends the deployment
    // name); the manifest-derived default is unchanged without it.
    #[test]
    fn program_name_sets_argv0() -> Result<()> {
        let _gate = engine_gate();
        fixture::RT.block_on(async {
            let deployment = DeploymentBuilder::new()
                .dynamic()
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

    // A `command = true` mark on a static `[[guest]]` entry is a registry
    // hit — no resolver consulted, the marked guest runs.
    #[test]
    fn static_hit() -> Result<()> {
        let _gate = engine_gate();
        fixture::RT.block_on(async {
            let wasm = find_guest("cli_wasm.wasm");
            let (resolver, calls) = CommandResolver::new(|| Ok(Some(cli_artifact()?)));
            let builder = DeploymentBuilder::new()
                .manifest(Manifest::new().guest(GuestEntry::new("app", wasm).command()))
                .args(vec!["add".to_owned(), "2".to_owned(), "40".to_owned()])
                .mode(Mode::Command)
                .resolver(resolver)
                .precompiled();
            // SAFETY: `find_guest` only returns artifacts this workspace built and
            // serialized itself (`cargo make test-guests`).
            let status = unsafe { run_precompiled::<(), EmptyWiring>(builder) }.await?;
            assert_eq!(status.code(), 0, "the marked command guest runs");
            assert_eq!(calls.load(Ordering::SeqCst), 0, "a registry hit never resolves");
            Ok(())
        })
    }

    // Two static `wasi:cli/run` exporters are ambiguous without a mark; the
    // same deployment with one entry marked `command = true` routes the run.
    #[test]
    fn mark_disambiguates_multiple_exporters() -> Result<()> {
        let _gate = engine_gate();
        fixture::RT.block_on(async {
            let wasm = find_guest("cli_wasm.wasm");
            let tail = &["add", "2", "40"];

            let unmarked = Manifest::new()
                .guest(GuestEntry::new("first", wasm.clone()))
                .guest(GuestEntry::new("second", wasm.clone()));
            let error = run_manifest(unmarked, tail)
                .await
                .expect_err("two unmarked exporters must be ambiguous");
            assert!(format!("{error:#}").contains("2 capable guests"), "{error:#}");

            let marked = Manifest::new()
                .guest(GuestEntry::new("first", wasm.clone()))
                .guest(GuestEntry::new("second", wasm).command());
            let status = run_manifest(marked, tail).await?;
            assert_eq!(status.code(), 0, "the marked guest runs");

            Ok(())
        })
    }
}
