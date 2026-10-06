//! The command façade's exit plumbing through the real runtime: the status a
//! `command!` guest's `Response` carries is the status the host observes.

#![cfg(not(target_arch = "wasm32"))]

use omnia_test::host::{Backends, Deployment};
use omnia_wasi_otel::WasiOtel;

test_programs::foreach_command!();

async fn exit_of(args: &[&str]) -> u8 {
    Deployment::new()
        .guest("cli", test_programs::COMMAND_EXIT_MAP)
        .args(args.iter().copied())
        .run(Backends::defaults().await, |deployment| {
            deployment.host::<WasiOtel, Backends>()?;
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
    assert_eq!(exit_of(&["bogus"]).await, 64, "an unknown verb is a usage error");
    assert_eq!(exit_of(&["--format", "json", "missing"]).await, 2, "the exit ignores the format");
}

// A captured deployment answers what the guest wrote beside its status: the
// rendered answer on stdout, the refusal's envelope on stderr, and each
// stream empty where the guest wrote nothing to it.
#[tokio::test]
async fn command_captured() {
    async fn captured(args: &[&str]) -> omnia_test::host::Run {
        Deployment::new()
            .guest("cli", test_programs::COMMAND_EXIT_MAP)
            .args(args.iter().copied())
            .captured()
            .run(Backends::defaults().await, |deployment| {
                deployment.host::<WasiOtel, Backends>()?;
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
