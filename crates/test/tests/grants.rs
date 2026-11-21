//! Per-guest grants through the real runtime: a guest sees the variables and
//! mounts its grants list, and one importing a package outside its
//! `interfaces` is refused at admission.

#![cfg(not(target_arch = "wasm32"))]

use omnia::{ExitStatus, Grants, GuestEntry};
use omnia_test::host::{Backends, Deployment, scratch};
use omnia_wasi_otel::WasiOtel;

test_programs::foreach_grants!();

// The deployment inherits more variables than the guest's grants reach and
// mounts more directories than they name. The guest loads at its first use,
// so the grants are its entry's, read at that admission.
#[tokio::test]
async fn grants_reach() {
    let (data, logs) = (scratch(), scratch());
    let package = std::env::var("CARGO_PKG_NAME").expect("cargo sets `CARGO_PKG_NAME`");
    let grants = Grants::none().env(["CARGO_PKG_NAME"]).mounts(["data"]);

    let status = Deployment::new()
        .mount(data.mount_as("data", false))
        .mount(logs.mount_as("logs", true))
        .inherit(["CARGO_MANIFEST_DIR", "CARGO_PKG_*"])
        .entry(GuestEntry::new("reach", test_programs::GRANTS_REACH).command().grants(grants))
        .args([format!("CARGO_PKG_NAME={package}"), "data".to_owned()])
        .run(Backends::defaults().await, |deployment| {
            deployment.host::<WasiOtel>()?;
            Ok(())
        })
        .await
        .expect("deployment runs");
    assert_eq!(status, ExitStatus::SUCCESS, "the guest's assertions held");
}

// Embedded bytes load at boot, so the refusal fails assembly before any
// guest runs.
#[tokio::test]
async fn undeclared_interface() {
    let bytes = std::fs::read(test_programs::GRANTS_REACH).expect("reading the guest");
    let grants = Grants::all().interfaces(["omnia:otel"]);

    let booted = Deployment::new()
        .entry(GuestEntry::new("reach", bytes).command().grants(grants))
        .boot(Backends::defaults().await, |deployment| {
            deployment.host::<WasiOtel>()?;
            Ok(())
        })
        .await;
    let Err(error) = booted else {
        panic!("the guest imports `wasi:` packages its grants omit");
    };
    let message = format!("{error:#}");
    assert!(message.contains("guest `reach` imports `wasi:"), "{message}");
    assert!(message.contains("to its `interfaces`"), "{message}");
}
