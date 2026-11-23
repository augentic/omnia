//! The command façade's exit plumbing through the real runtime: the status a
//! `command!` guest's `Response` carries is the status the host observes. A
//! guest that blocks on the host also runs in a pool of one instance.

#![cfg(not(target_arch = "wasm32"))]

use omnia::{ExitStatus, RuntimeOptions};
use omnia_test::host::{Backends, Deployment};
use omnia_wasi_otel::WasiOtel;

test_programs::foreach_command!();

async fn exit_of(args: &[&str]) -> u8 {
    Deployment::new()
        .guest("cli", test_programs::COMMAND_EXIT_MAP)
        .args(args.iter().copied())
        .run(Backends::defaults().await, |deployment| {
            deployment.host::<WasiOtel>()?;
            Ok(())
        })
        .await
        .expect("deployment runs")
        .code_u8()
}

#[tokio::test]
async fn command_exit_map() {
    assert_eq!(exit_of(&["ok"]).await, 0);
    assert_eq!(exit_of(&["bad"]).await, 1);
    assert_eq!(exit_of(&["missing"]).await, 2);
    assert_eq!(exit_of(&["upstream"]).await, 4);
    assert_eq!(exit_of(&["taken"]).await, 5);
    assert_eq!(exit_of(&["bogus"]).await, 64, "an unknown verb is a usage error");
    assert_eq!(exit_of(&["--format", "json", "missing"]).await, 2, "the exit ignores the format");
}

// One instance derives a pool of two stacks, one memory, and four core
// instances and tables. A core-instance total of one refuses the guest, so
// the options set in code reach the pool.
#[tokio::test]
async fn command_blocking() {
    async fn run(options: RuntimeOptions) -> anyhow::Result<ExitStatus> {
        Deployment::new()
            .guest("cli", test_programs::COMMAND_BLOCKING)
            .args(["wait"])
            .options(options)
            .run(Backends::defaults().await, |deployment| {
                deployment.host::<WasiOtel>()?;
                Ok(())
            })
            .await
    }

    let one = RuntimeOptions {
        pool_max_instances: 1,
        ..RuntimeOptions::default()
    };
    let status = run(one.clone()).await.expect("the guest runs in a pool of one instance");
    assert_eq!(status, ExitStatus::SUCCESS);

    let starved = RuntimeOptions {
        pool_total_core_instances: Some(1),
        ..one
    };
    let err = run(starved).await.expect_err("one core instance is too few for the guest");
    assert!(format!("{err:#}").contains("core instances"), "unexpected error: {err:#}");
}

// A captured deployment answers what the guest wrote beside its status: the
// rendered answer on stdout, the refusal's envelope on stderr, and each
// stream empty where the guest wrote nothing to it.
#[tokio::test]
async fn captured_run() {
    async fn captured(args: &[&str]) -> omnia_test::host::Run {
        Deployment::new()
            .guest("cli", test_programs::COMMAND_EXIT_MAP)
            .args(args.iter().copied())
            .captured()
            .run(Backends::defaults().await, |deployment| {
                deployment.host::<WasiOtel>()?;
                Ok(())
            })
            .await
            .expect("deployment runs")
    }

    let run = captured(&["ok"]).await;
    assert_eq!(run.status.code_u8(), 0);
    assert_eq!(run.stdout, "ok\n");
    assert_eq!(run.stderr, "", "a success writes nothing to stderr");

    let run = captured(&["--format", "json", "missing"]).await;
    assert_eq!(run.status.code_u8(), 2);
    assert_eq!(run.stdout, "", "a refusal writes nothing to stdout");
    assert!(run.stderr.contains("refused as not found"), "the envelope: {}", run.stderr);
}

// `command::export!` stamps the guest crate's version and description where
// the router set none; `test-programs` shares the workspace version.
#[tokio::test]
async fn package_defaults() {
    async fn captured(args: &[&str]) -> omnia_test::host::Run {
        Deployment::new()
            .guest("cli", test_programs::COMMAND_EXIT_MAP)
            .args(args.iter().copied())
            .captured()
            .run(Backends::defaults().await, |deployment| {
                deployment.host::<WasiOtel>()?;
                Ok(())
            })
            .await
            .expect("deployment runs")
    }

    let run = captured(&["--version"]).await;
    assert_eq!(run.status.code_u8(), 0);
    assert_eq!(run.stdout, format!("exit-map {}\n", env!("CARGO_PKG_VERSION")));

    let run = captured(&["--help"]).await;
    assert_eq!(run.status.code_u8(), 0);
    assert!(
        run.stdout.starts_with("Guest scenario programs driven by the e2e test suites"),
        "the crate description opens help: {}",
        run.stdout
    );
}
