//! Bytes-sourced (embedded) guest acquisition seam.
//!
//! A `[[guest]]` entry may carry component bytes instead of a path — the
//! `include_bytes!` embedding shape (`GuestEntry::new(id, bytes)`). These
//! tests read the pre-built `guest-link` artifacts at run time (tests never
//! invoke Cargo) and prove a bytes-sourced guest loads, registers, and
//! dispatches exactly like a path-sourced one, and that the artifact-trust
//! policy applies to bytes as it does to paths.

use std::sync::Arc;

use anyhow::{Context as _, Result, bail, ensure};
use omnia::wasmtime::component::Val;
use omnia::{
    DeploymentBuilder, GuestEntry, GuestId, Manifest, MountRegistry, Runtime, serve_links,
};
use omnia_testkit::find_guest;

use crate::fixture;

type TestCtx = omnia::StoreCtx<()>;

/// Read the raw `.wasm` sibling for `file` (never the serialized `.bin`), so
/// the safe (`WasmOnly`) build genuinely exercises the raw-bytes path.
fn wasm_bytes(file: &str) -> Result<Vec<u8>> {
    let path = find_guest(file).with_extension("wasm");
    std::fs::read(&path).with_context(|| format!("reading guest {}", path.display()))
}

/// Read the serialized `.bin` for `file`, failing fast when it is missing so
/// the pre-compiled path is genuinely exercised.
fn precompiled_bytes(file: &str) -> Result<Vec<u8>> {
    let path = find_guest(file);
    ensure!(
        path.extension().is_some_and(|ext| ext == "bin"),
        "{} has no serialized .bin sibling; run `cargo make test-guests`",
        path.display()
    );
    std::fs::read(&path).with_context(|| format!("reading guest {}", path.display()))
}

/// Build the two-guest link deployment with the router sourced from bytes and
/// the responder from a path, proving the two source kinds mix in one
/// manifest.
async fn build_runtime() -> Result<Runtime<()>> {
    let responder = find_guest("guest_link_responder_wasm.wasm").with_extension("wasm");
    let router = wasm_bytes("guest_link_router_wasm.wasm")?;

    let manifest = Manifest::new()
        .guest(GuestEntry::new("responder", responder))
        .guest(GuestEntry::new("router", router).link("omnia:link/echo"));

    // Raw wasm bytes load under the safe (WasmOnly) build — no attestation.
    let deployment = DeploymentBuilder::new()
        .manifest(manifest)
        .build::<TestCtx>()
        .await
        .context("building deployment")?;
    let registry = deployment.into_registry().context("assembling registry")?;
    let runtime = Runtime::<()>::from_parts(
        Arc::new(registry),
        Vec::new(),
        Arc::new(MountRegistry::default()),
        (),
    );
    serve_links(&runtime).await.context("wiring link serve side")?;
    Ok(runtime)
}

/// Instantiate the bytes-sourced router fresh and drive `run(message)`.
async fn call_router(runtime: &Runtime<()>, message: &str) -> Result<String> {
    let guest =
        runtime.registry().get(&GuestId::from("router")).context("router guest is registered")?;
    let mut store = runtime.build_store(runtime.store());
    let instance = runtime
        .instantiate(guest.instance_pre(), &mut store)
        .await
        .context("instantiating router")?;
    let run = instance.get_func(&mut store, "run").context("router exports `run`")?;

    let mut results = vec![Val::Bool(false)];
    run.call_async(&mut store, &[Val::String(message.to_owned())], &mut results)
        .await
        .map_err(anyhow::Error::from)
        .context("calling router.run")?;

    match results.into_iter().next() {
        Some(Val::String(echoed)) => Ok(echoed),
        other => bail!("router.run returned a non-string result: {other:?}"),
    }
}

// A bytes-sourced guest dispatches like a path-sourced one: the embedded
// router reaches the path-sourced responder over the host-mediated link.
#[test]
fn bytes_sourced_guest_dispatches() -> Result<()> {
    fixture::RT.block_on(async {
        let runtime = build_runtime().await?;
        let echoed = call_router(&runtime, "hello").await?;
        assert_eq!(echoed, "responder echoes: hello");
        Ok(())
    })
}

// Pre-compiled bytes follow the same trust policy as pre-compiled paths: the
// safe build rejects them; the `precompiled()` unsafe build admits them.
#[test]
fn precompiled_bytes_gated_by_artifact_policy() -> Result<()> {
    fixture::RT.block_on(async {
        let bytes = precompiled_bytes("guest_link_responder_wasm.wasm")?;

        let manifest = Manifest::new().guest(GuestEntry::new("responder", bytes.clone()));
        let error = DeploymentBuilder::new()
            .manifest(manifest)
            .build::<TestCtx>()
            .await
            .err()
            .context("the safe build must reject pre-compiled bytes")?;
        ensure!(
            format!("{error:#}").contains("pre-compiled"),
            "rejection names the artifact kind: {error:#}"
        );

        let manifest = Manifest::new().guest(GuestEntry::new("responder", bytes));
        let builder = DeploymentBuilder::new().manifest(manifest).precompiled();
        // SAFETY: the bytes were built and serialized by this workspace's own
        // `cargo make test-guests` pipeline (omnia's compile path).
        let deployment =
            unsafe { builder.build::<TestCtx>() }.await.context("building deployment")?;
        deployment.into_registry().context("assembling registry")?;
        Ok(())
    })
}
