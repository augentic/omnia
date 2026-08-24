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
//! to the marked entry past a second `wasi:cli/run` exporter that would
//! otherwise be ambiguous, and `argv[0]` via `program_name`.

// The serialized `.bin` guests are workspace-built (`cargo make test-guests`),
// satisfying the unsafe pre-compiled build/registration contracts.
#![allow(unsafe_code)]

use std::path::Path;

use anyhow::{Context as _, Result};
use omnia::{
    Backend as _, Backends, Deployment, DeploymentBuilder, ExitStatus, Manifest, Mode, Runtime,
    StoreCtx, Wiring, run_precompiled,
};
use omnia_abi_tests::find_guest;
use omnia_wasi_otel::{HasOtel, OtelDefault, WasiOtel, WasiOtelCtx};

/// The `omnia-guest` command router instruments dispatch over `wasi:otel`, so
/// every router-based guest imports it; the bundle links the no-op default.
#[derive(Clone)]
struct Bundle {
    otel: OtelDefault,
}

impl Backends for Bundle {
    async fn connect() -> Result<Self> {
        Ok(Self {
            otel: OtelDefault::connect().await?,
        })
    }
}

impl HasOtel for Bundle {
    fn otel_ctx(&mut self) -> &mut dyn WasiOtelCtx {
        &mut self.otel
    }
}

struct OtelWiring;

impl Wiring<Bundle> for OtelWiring {
    fn link(deployment: &mut Deployment<StoreCtx<Bundle>>) -> Result<()> {
        deployment.host::<WasiOtel, Bundle>()?;
        Ok(())
    }

    fn serve(_runtime: &Runtime<Bundle>) -> impl std::future::Future<Output = Result<()>> + Send {
        std::future::ready(Ok(()))
    }
}

/// Drive `wasi:cli/run` once with `tail` guest argv (the program name is
/// prepended by command mode) and return the guest's exit status.
async fn run_cli(wasm: &Path, tail: &[&str]) -> Result<ExitStatus> {
    // `wasi:cli` is wired by the deployment builder, and `Runtime::new`
    // threads the guest argv into every store.
    let builder = DeploymentBuilder::new()
        .manifest(Manifest::from_wasm(wasm))
        .args(tail.iter().map(|arg| (*arg).to_string()).collect::<Vec<_>>())
        .mode(Mode::Command)
        .precompiled();
    // SAFETY: `find_guest` only returns artifacts this workspace built and
    // serialized itself (`cargo make test-guests`).
    unsafe { run_precompiled::<Bundle, OtelWiring>(builder) }.await.context("running command")
}

#[tokio::test(flavor = "multi_thread")]
async fn exit_codes() -> Result<()> {
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
}

mod command_flag {
    use omnia::GuestEntry;

    use super::*;

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
        unsafe { run_precompiled::<Bundle, OtelWiring>(builder) }.await
    }

    // `program_name` overrides `argv[0]` (command mode prepends the deployment
    // name); the manifest-derived default is unchanged without it.
    #[tokio::test(flavor = "multi_thread")]
    async fn program_name_sets_argv0() -> Result<()> {
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
    }

    // Two static `wasi:cli/run` exporters are ambiguous without a mark; the
    // same deployment with one entry marked `command = true` routes the run.
    #[tokio::test(flavor = "multi_thread")]
    async fn mark_disambiguates_multiple_exporters() -> Result<()> {
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
    }
}
