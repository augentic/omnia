//! The guest's `wasi:cli` environment through the real runtime: `RUST_LOG`
//! is the deployment's tracing directives — the level selected for the run
//! (else the process's own, else the command-mode default) composed with the
//! process `RUST_LOG`'s targeted directives.

#![cfg(not(target_arch = "wasm32"))]

use omnia::{ExitStatus, LevelFilter, telemetry};
use omnia_test::host::{Backends, Deployment};
use omnia_wasi_otel::WasiOtel;

test_programs::foreach_cli!();

async fn run(deployment: Deployment, expected: String) -> ExitStatus {
    deployment
        .guest("cli", test_programs::CLI_RUST_LOG)
        .args([expected])
        .run(Backends::defaults().await, |deployment| {
            deployment.host::<WasiOtel, Backends>()?;
            Ok(())
        })
        .await
        .expect("deployment runs")
}

// The expectation the runtime composes for a command run: the test process's
// `RUST_LOG` is read, never set, so the suite never writes its own
// environment. A set variable stands (its bare level displaced by a selected
// one), an unset one falls back to the command-mode `info`.
fn expected(level: Option<LevelFilter>) -> String {
    telemetry::directives(level, LevelFilter::INFO, std::env::var("RUST_LOG").ok().as_deref())
}

#[tokio::test]
async fn cli_rust_log() {
    assert_eq!(run(Deployment::new(), expected(None)).await, ExitStatus::SUCCESS);
}

#[tokio::test]
async fn cli_rust_log_level() {
    let deployment = Deployment::new().level(LevelFilter::DEBUG);
    let expected = expected(Some(LevelFilter::DEBUG));
    assert!(expected.starts_with("debug"), "the selected level leads: {expected}");
    assert_eq!(run(deployment, expected).await, ExitStatus::SUCCESS);
}

#[test]
fn directives_and_bare() {
    // Flag displaces the bare level while preserving targeted directives.
    assert_eq!(
        telemetry::directives(Some(LevelFilter::DEBUG), LevelFilter::INFO, Some("info,tower=off")),
        "debug,tower=off"
    );
    assert_eq!(
        telemetry::directives(None, LevelFilter::INFO, Some("warn,omnia_core=debug")),
        "warn,omnia_core=debug"
    );
    // Unset falls back; targeted-only takes no fallback.
    assert_eq!(telemetry::directives(None, LevelFilter::INFO, None), "info");
    assert_eq!(
        telemetry::directives(None, LevelFilter::INFO, Some("omnia_core=trace")),
        "omnia_core=trace"
    );
    assert_eq!(
        telemetry::bare(Some(LevelFilter::INFO), LevelFilter::WARN, Some("trace")),
        Some(LevelFilter::INFO)
    );
    assert_eq!(telemetry::bare(None, LevelFilter::WARN, Some("my_sdk=debug")), None);
}

#[test]
fn cli_grammar_and_mounts() {
    use std::path::PathBuf;

    use omnia::{Cli, Command, Parser as _};

    let parsed = Cli::try_parse_from([
        "bin",
        "-vv",
        "run",
        "guest.wasm",
        "-m",
        "omnia.toml",
        "--mount",
        "workspace,writable",
        "--",
        "-q",
    ])
    .expect("cli grammar parses");
    assert_eq!((parsed.verbose, parsed.quiet), (2, 0));
    let Command::Run {
        wasm,
        manifest,
        mounts,
        args,
    } = parsed.command
    else {
        panic!("expected `run` command");
    };
    assert_eq!(wasm, Some(PathBuf::from("guest.wasm")));
    assert_eq!(manifest, Some(PathBuf::from("omnia.toml")));
    assert_eq!(args, ["-q"]);
    assert_eq!(mounts.len(), 1);
    assert_eq!(mounts[0].name, ".");
    assert_eq!(mounts[0].host_path, PathBuf::from("workspace"));
    assert!(mounts[0].writable);

    assert!(
        Cli::try_parse_from(["bin", "run", "-v", "-q", "guest.wasm"]).is_err(),
        "`-v` with `-q` is a usage error"
    );
}
