//! End-to-end tests for host-mediated link dispatch: every scenario drives
//! real guest components from `crates/test-programs` through the omnia
//! runtime. `partial` imports a strict subset of the `omnia-test:link/ops`
//! functions `full` imports (the componentizer prunes unused imports), so the
//! suite proves the shared linker unions per-guest imports at function
//! granularity — at bootstrap and for late registration.

#![cfg(not(target_arch = "wasm32"))]

use std::time::Duration;

use anyhow::{Context as _, Result, bail};
use omnia::wasmtime::component::Val;
use omnia::{
    ChainCtx, DeploymentBuilder, GuestArtifact, GuestEntry, GuestId, Manifest, Runtime, StoreCtx,
};

// Every guest program in `crates/test-programs` must have a matching test
// here; a new program without one fails to compile.
test_programs::foreach_link!();

/// Boot a runtime over `guests` (assembled in order); nothing declares the
/// `omnia-test:link/ops` seam, which is read off the components.
async fn boot(guests: &[(&str, &str)]) -> Result<Runtime<()>> {
    boot_with(guests, |builder| builder).await
}

/// `boot` with `configure` applied to the builder (dispatch depth, timeout).
async fn boot_with(
    guests: &[(&str, &str)], configure: impl FnOnce(DeploymentBuilder) -> DeploymentBuilder,
) -> Result<Runtime<()>> {
    let mut manifest = Manifest::new();
    for (name, wasm) in guests {
        manifest = manifest.guest(GuestEntry::new(*name, *wasm));
    }
    let deployment = configure(DeploymentBuilder::new().manifest(manifest))
        .build::<StoreCtx<()>>()
        .await
        .context("building deployment")?;
    deployment.assemble(()).await
}

/// Instantiate `guest` fresh and drive its exported `func` with one string
/// argument, returning the string result.
async fn call(runtime: &Runtime<()>, guest: &str, func: &str, message: &str) -> Result<String> {
    let entry = runtime
        .registry()
        .get(&GuestId::from(guest))
        .with_context(|| format!("guest `{guest}` is not registered"))?;
    let mut store = runtime.build_store(runtime.store());
    let instance = runtime
        .instantiate(entry.instance_pre(), &mut store)
        .await
        .with_context(|| format!("instantiating `{guest}`"))?;
    let export = instance
        .get_func(&mut store, func)
        .with_context(|| format!("guest `{guest}` exports `{func}`"))?;

    // `call_async` drives sync- and async-lifted exports alike.
    let mut results = vec![Val::Bool(false)];
    export
        .call_async(&mut store, &[Val::String(message.to_owned())], &mut results)
        .await
        .map_err(anyhow::Error::from)
        .with_context(|| format!("calling `{guest}`'s `{func}`"))?;
    match results.into_iter().next() {
        Some(Val::String(answer)) => Ok(answer),
        other => bail!("`{guest}`'s `{func}` returned a non-string result: {other:?}"),
    }
}

#[tokio::test]
async fn link_echoer() {
    let runtime =
        boot(&[("echoer", test_programs::LINK_ECHOER), ("full", test_programs::LINK_FULL)])
            .await
            .expect("deployment boots");

    // Both dispatch paths round-trip to the exporter.
    let sync = call(&runtime, "full", "poke", "hi").await.expect("sync dispatch");
    assert_eq!(sync, "echoer pong: hi");
    let concurrent = call(&runtime, "full", "poke-async", "hi").await.expect("async dispatch");
    assert_eq!(concurrent, "echoer pong-async: hi");
}

#[tokio::test]
async fn link_partial() {
    let runtime =
        boot(&[("echoer", test_programs::LINK_ECHOER), ("partial", test_programs::LINK_PARTIAL)])
            .await
            .expect("deployment boots");

    // An interface wired with only one of its functions still resolves.
    let answer = call(&runtime, "partial", "poke", "hi").await.expect("subset dispatch");
    assert_eq!(answer, "echoer pong: hi");
}

// Nothing declares the seam, so an import no guest exports is relayed rather
// than left unresolved: the deployment boots, and the call fails when the
// relay finds no target.
#[tokio::test]
async fn link_unserved() {
    let runtime = boot(&[("full", test_programs::LINK_FULL)]).await.expect("deployment boots");

    let err = call(&runtime, "full", "poke", "hi").await.expect_err("no guest serves `ops`");
    assert!(format!("{err:#}").contains("`echoer` is not registered"), "unexpected error: {err:#}");
}

// A guest that exports no interface outside the host's namespaces parks no
// route, so a call dispatched to it names the missing export rather than a
// missing guest.
#[tokio::test]
async fn link_unlinked_target() {
    let runtime = boot(&[("echoer", test_programs::LINK_FULL), ("full", test_programs::LINK_FULL)])
        .await
        .expect("deployment boots");

    let err = call(&runtime, "full", "poke", "hi").await.expect_err("`echoer` exports no `ops`");
    assert!(
        format!("{err:#}").contains("registered but exports no linked interface"),
        "unexpected error: {err:#}"
    );
}

// The union regression: `partial` assembles first and wires only `ping`, so
// the linker must reopen the interface and add `full`'s `ping-async` rather
// than skip the already-seen interface (which failed `full`'s
// pre-instantiation before wiring went per-function).
#[tokio::test]
async fn link_full() {
    let runtime = boot(&[
        ("echoer", test_programs::LINK_ECHOER),
        ("partial", test_programs::LINK_PARTIAL),
        ("full", test_programs::LINK_FULL),
    ])
    .await
    .expect("subset-first deployment boots");

    let subset = call(&runtime, "partial", "poke", "one").await.expect("subset dispatch");
    assert_eq!(subset, "echoer pong: one");
    let sync = call(&runtime, "full", "poke", "two").await.expect("sync dispatch");
    assert_eq!(sync, "echoer pong: two");
    let concurrent = call(&runtime, "full", "poke-async", "three").await.expect("async dispatch");
    assert_eq!(concurrent, "echoer pong-async: three");
}

// The late dual of `link_full`: bootstrap wires only `partial`'s subset, so
// registering `full` afterwards must polyfill the missing `ping-async` on the
// linker clone.
#[tokio::test]
async fn link_full_registered_late() {
    let runtime =
        boot(&[("echoer", test_programs::LINK_ECHOER), ("partial", test_programs::LINK_PARTIAL)])
            .await
            .expect("deployment boots");

    let wasm = std::fs::read(test_programs::LINK_FULL).expect("reading full guest artifact");
    runtime.register("full", GuestArtifact::wasm(wasm)).await.expect("late registration");

    let sync = call(&runtime, "full", "poke", "late").await.expect("sync dispatch");
    assert_eq!(sync, "echoer pong: late");
    let concurrent = call(&runtime, "full", "poke-async", "late").await.expect("async dispatch");
    assert_eq!(concurrent, "echoer pong-async: late");
    // The bootstrap guest is untouched by the late wiring.
    let subset = call(&runtime, "partial", "poke", "still").await.expect("subset dispatch");
    assert_eq!(subset, "echoer pong: still");
}

// The relay takes the id `echoer` because `full` hard-codes `ping("echoer",
// ..)` and the default selector routes on that argument; every relay hop then
// re-dispatches to itself, consuming one depth unit per hop.
#[tokio::test]
async fn link_relay() {
    let runtime = boot_with(
        &[("echoer", test_programs::LINK_RELAY), ("full", test_programs::LINK_FULL)],
        |builder| builder.max_dispatch_depth(3),
    )
    .await
    .expect("deployment boots");

    // `full` → relay (depth 1) → relay (depth 2): within the bound.
    let answer = call(&runtime, "full", "poke", "1").await.expect("two-hop chain");
    assert_eq!(answer, "echoer relayed to the end");

    // The relay's own hop trips the bound, and the callee's trap propagates
    // through the caller's polyfill with its text intact.
    let err = call(&runtime, "full", "poke", "5").await.expect_err("chain exceeds the bound");
    assert!(format!("{err:#}").contains("exceeds maximum"), "unexpected error: {err:#}");
}

// The sleeper takes the id `echoer` for the same reason as the relay. Only
// the caller is awaited, so the test finishes well inside the 2 s the sleeper
// would otherwise hold its store.
#[tokio::test]
async fn link_sleeper() {
    let runtime = boot_with(
        &[("echoer", test_programs::LINK_SLEEPER), ("full", test_programs::LINK_FULL)],
        |builder| builder.guest_timeout(Duration::from_millis(50)),
    )
    .await
    .expect("deployment boots");

    let err = call(&runtime, "full", "poke", "sleep").await.expect_err("callee outlives the bound");
    assert!(format!("{err:#}").contains("timed out"), "unexpected error: {err:#}");

    // The target is still reachable after the timed-out call was abandoned.
    let answer = call(&runtime, "full", "poke", "awake").await.expect("dispatch after timeout");
    assert_eq!(answer, "echoer woke: awake");
}

// The skewed exporter takes the id `echoer` so `full`'s wired import of
// `ping` is the one it is checked against; the mismatch is refused when the
// exporter is served, so assembly fails before any call.
#[tokio::test]
async fn link_skewed() {
    let Err(err) =
        boot(&[("echoer", test_programs::LINK_SKEWED), ("full", test_programs::LINK_FULL)]).await
    else {
        panic!("skewed exporter was served");
    };

    let text = format!("{err:#}");
    for needle in ["echoer", "full", "omnia-test:link/ops", "ping"] {
        assert!(text.contains(needle), "`{needle}` missing from: {text}");
    }
}

// The host→guest hop is entered from a root, so it is depth 1 and with a bound
// of 3 the relay may hop twice more: `2` lands exactly on the bound, `3` would
// need depth 4. Were the dispatcher to restart the chain at 0, `3` would
// succeed.
#[tokio::test]
async fn relay_via_dispatcher() {
    let runtime = boot_with(
        &[("echoer", test_programs::LINK_RELAY), ("full", test_programs::LINK_FULL)],
        |builder| builder.max_dispatch_depth(3),
    )
    .await
    .expect("deployment boots");

    let dispatch = |hops: &str| {
        runtime.dispatcher().invoke(
            ChainCtx::server(),
            GuestId::from("echoer"),
            Some("omnia-test:link/ops".into()),
            "ping".into(),
            vec![Val::String("echoer".into()), Val::String(hops.to_owned())],
        )
    };

    let answer = dispatch("2").await.expect("chain within the bound");
    assert_eq!(answer, vec![Val::String("echoer relayed to the end".into())]);

    // The over-bound hop fails inside a guest polyfill; its trap propagates
    // back through every fresh callee to the dispatcher's caller.
    let err = dispatch("3").await.expect_err("chain exceeds the bound");
    assert!(format!("{err:#}").contains("exceeds maximum"), "unexpected error: {err:#}");
}
