//! End-to-end tests for the `omnia:plugins/loader` host capability: a real
//! requester guest from `crates/test-programs` drives loads through omnia's
//! runtime, with `MountAcquire` reading components staged in a scratch mount.
//! The requester asserts internally (handles, digests, dispatch answers, and
//! every typed refusal); the host side stages artifacts and checks the exit.

#![cfg(not(target_arch = "wasm32"))]

use anyhow::{Context as _, Result};
use omnia::{
    DeploymentBuilder, ExitStatus, GuestEntry, Manifest, Mode, MountAcquire, Runtime, StoreCtx,
    WasiPlugins,
};

// Every guest program in `crates/test-programs/programs/plugins` must have a
// matching test here; a new program without one fails to compile.
test_utils::foreach_plugins!();

/// Drive `wasm` as the `requester` command guest: the scratch dir mounts at
/// `.`, `omnia-test:link/ops` is the declared plugin seam, and `MountAcquire`
/// is the compiled-in acquirer.
async fn run_requester(wasm: &str, scratch: &test_utils::Scratch) -> Result<ExitStatus> {
    let manifest = Manifest::new()
        .plugins(["omnia-test:link/ops"])
        .guest(GuestEntry::new("requester", wasm))
        .mounts([scratch.mount(false)]);
    let deployment = DeploymentBuilder::new()
        .manifest(manifest)
        .mode(Mode::Command)
        .acquirer(MountAcquire)
        .build::<StoreCtx<()>>()
        .await
        .context("building deployment")?;
    let runtime = Runtime::new(deployment, |deployment| {
        deployment.host::<WasiPlugins, ()>()?;
        Ok(())
    })
    .await
    .context("assembling runtime")?;
    runtime.run_command().await
}

#[tokio::test]
async fn plugins_load_path() {
    let scratch = test_utils::scratch("plugins_load_path");
    std::fs::copy(test_utils::LINK_ECHOER, scratch.path().join("plugin.wasm"))
        .expect("staging the loadable echoer");

    let status =
        run_requester(test_utils::PLUGINS_LOAD_PATH, &scratch).await.expect("deployment runs");
    assert_eq!(status, ExitStatus::SUCCESS, "the requester's assertions all held");
}

#[tokio::test]
async fn plugins_load_refused() {
    let scratch = test_utils::scratch("plugins_load_refused");
    std::fs::copy(test_utils::LINK_ECHOER, scratch.path().join("plugin.wasm"))
        .expect("staging the loadable echoer");
    // Exports no `omnia-test:link/ops` instance — the seam-missing target.
    std::fs::copy(test_utils::LINK_FULL, scratch.path().join("noseam.wasm"))
        .expect("staging the seamless component");
    // Leading ELF magic is exactly what the loader sniffs; the tail is junk,
    // proving refusal happens before any wasmtime parsing.
    std::fs::write(scratch.path().join("native.bin"), [0x7f, b'E', b'L', b'F', 0, 0, 0, 0])
        .expect("staging native bytes");

    let status =
        run_requester(test_utils::PLUGINS_LOAD_REFUSED, &scratch).await.expect("deployment runs");
    assert_eq!(status, ExitStatus::SUCCESS, "the requester's assertions all held");
}
