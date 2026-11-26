//! End-to-end tests for host-mediated link dispatch: every scenario drives
//! real guest components from `crates/test-programs` through the omnia
//! runtime. `partial` imports a strict subset of the `omnia-test:link/ops`
//! functions `full` imports (the componentizer prunes unused imports), so the
//! suite proves the shared linker unions per-guest imports at function
//! granularity — at bootstrap and for late registration.

#![cfg(not(target_arch = "wasm32"))]

use std::path::Path;
use std::time::Duration;

use anyhow::{Context as _, Result, bail};
use omnia::wasmtime::Enabled;
use omnia::wasmtime::component::Val;
use omnia::{
    ChainCtx, ChainPolicy, CompileOptions, DeploymentBuilder, Digest, Grants, GuestEntry,
    GuestError, GuestId, Manifest, Mode, Runtime, RuntimeOptions, Transport, TransportKind,
    Verified,
};

// Every guest program in `crates/test-programs` must have a matching test
// here; a new program without one fails to compile.
test_programs::foreach_link!();

// each guest is read into bytes so it loads at boot, as a `runtime!`'s
// embedded guests do; the link seam is read off the components
async fn boot(guests: &[(&str, &str)]) -> Result<Runtime<()>> {
    boot_with(guests, |builder| builder).await
}

async fn boot_with(
    guests: &[(&str, &str)], configure: impl FnOnce(DeploymentBuilder) -> DeploymentBuilder,
) -> Result<Runtime<()>> {
    let mut manifest = Manifest::new();
    for (name, wasm) in guests {
        let bytes = std::fs::read(wasm).with_context(|| format!("reading {wasm}"))?;
        manifest = manifest.guest(GuestEntry::new(*name, bytes));
    }
    assemble(manifest, configure).await
}

async fn assemble(
    manifest: Manifest, configure: impl FnOnce(DeploymentBuilder) -> DeploymentBuilder,
) -> Result<Runtime<()>> {
    let deployment = configure(DeploymentBuilder::new().manifest(manifest))
        .build::<()>()
        .await
        .context("building deployment")?;
    deployment.assemble(()).await
}

// instantiates `guest` fresh, loading it first when the deployment declares it
async fn call(runtime: &Runtime<()>, guest: &str, func: &str, message: &str) -> Result<String> {
    let entry = runtime
        .guest(&GuestId::from(guest))
        .await
        .with_context(|| format!("resolving guest `{guest}`"))?;
    let results = runtime
        .lease(entry)
        .await
        .invoke(async |mut entered, instance, _bound| {
            let export = instance
                .get_func(&mut entered, func)
                .with_context(|| format!("guest `{guest}` exports `{func}`"))?;

            // `call_async` drives sync- and async-lifted exports alike.
            let mut results = vec![Val::Bool(false)];
            export
                .call_async(&mut entered, &[Val::String(message.to_owned())], &mut results)
                .await
                .map_err(anyhow::Error::from)
                .with_context(|| format!("calling `{guest}`'s `{func}`"))?;
            anyhow::Ok(results)
        })
        .await
        .with_context(|| format!("instantiating `{guest}`"))??;
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
async fn unserved_import() {
    let runtime = boot(&[("full", test_programs::LINK_FULL)]).await.expect("deployment boots");

    let err = call(&runtime, "full", "poke", "hi").await.expect_err("no guest serves `ops`");
    assert!(format!("{err:#}").contains("`echoer` is not registered"), "unexpected error: {err:#}");
}

// A path guest loads at its first use, and a link call is one: the call loads
// `echoer` through the runtime's first-use seam and runs on a fresh instance
// of it. The importer boots from bytes, so the seam it imports was polyfilled
// at bootstrap.
#[tokio::test]
async fn declared_exporter() {
    let full = std::fs::read(test_programs::LINK_FULL).expect("reading the full guest");
    let manifest = Manifest::new()
        .guest(GuestEntry::new("full", full))
        .guest(GuestEntry::new("echoer", test_programs::LINK_ECHOER));
    let runtime = assemble(manifest, |builder| builder).await.expect("deployment boots");
    assert!(runtime.registry().get(&GuestId::from("echoer")).is_none(), "absent until used");

    let sync = call(&runtime, "full", "poke", "hi").await.expect("sync dispatch");
    assert_eq!(sync, "echoer pong: hi");
    assert!(runtime.registry().get(&GuestId::from("echoer")).is_some(), "the call loaded it");
    let concurrent = call(&runtime, "full", "poke-async", "hi").await.expect("async dispatch");
    assert_eq!(concurrent, "echoer pong-async: hi");

    // A declared guest that fails to load fails the call that named it.
    let manifest = Manifest::new()
        .guest(GuestEntry::new("full", std::fs::read(test_programs::LINK_FULL).expect("full")))
        .guest(GuestEntry::new("echoer", "/nonexistent/echoer.wasm"));
    let runtime = assemble(manifest, |builder| builder).await.expect("deployment boots");
    let err = call(&runtime, "full", "poke", "hi").await.expect_err("the exporter cannot load");
    assert!(format!("{err:#}").contains("/nonexistent/echoer.wasm"), "unexpected error: {err:#}");
}

// The target is registered but does not export the interface the call names,
// so the call names the missing export rather than a missing guest.
#[tokio::test]
async fn unlinked_target() {
    let runtime = boot(&[("echoer", test_programs::LINK_FULL), ("full", test_programs::LINK_FULL)])
        .await
        .expect("deployment boots");

    let err = call(&runtime, "full", "poke", "hi").await.expect_err("`echoer` exports no `ops`");
    assert!(
        format!("{err:#}").contains("guest `echoer` exports no interface `omnia-test:link/ops`"),
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
async fn late_registration() {
    let runtime =
        boot(&[("echoer", test_programs::LINK_ECHOER), ("partial", test_programs::LINK_PARTIAL)])
            .await
            .expect("deployment boots");

    let wasm = std::fs::read(test_programs::LINK_FULL).expect("reading full guest artifact");
    runtime.register("full", wasm, Grants::all()).await.expect("late registration");

    let sync = call(&runtime, "full", "poke", "late").await.expect("sync dispatch");
    assert_eq!(sync, "echoer pong: late");
    let concurrent = call(&runtime, "full", "poke-async", "late").await.expect("async dispatch");
    assert_eq!(concurrent, "echoer pong-async: late");
    // The bootstrap guest is untouched by the late wiring.
    let subset = call(&runtime, "partial", "poke", "still").await.expect("subset dispatch");
    assert_eq!(subset, "echoer pong: still");
}

// `register` is the embedder's untrusted-bytes path, so it admits raw wasm
// alone: `omnia compile` output is refused there, and reaches `admit` only
// through the `unsafe` `Verified::trusted`, the embedder's word that the
// artifact came from its own toolchain.
#[tokio::test]
async fn register_precompiled() {
    let runtime =
        boot(&[("echoer", test_programs::LINK_ECHOER), ("partial", test_programs::LINK_PARTIAL)])
            .await
            .expect("deployment boots");

    let artifact = Path::new(env!("CARGO_TARGET_TMPDIR")).join("full.cwasm");
    omnia::compile::compile(
        Path::new(test_programs::LINK_FULL),
        Some(artifact.clone()),
        None,
        &CompileOptions::default(),
    )
    .expect("compiling the full guest ahead of time");
    let bytes = std::fs::read(&artifact).expect("reading the compiled artifact");

    let err = runtime
        .register("full", bytes.clone(), Grants::all())
        .await
        .expect_err("pre-compiled is refused");
    assert!(format!("{err:#}").contains("`Verified::wasm`"), "unexpected error: {err:#}");
    assert!(runtime.registry().get(&GuestId::from("full")).is_none(), "nothing was admitted");

    #[expect(unsafe_code, reason = "the embedder's trusted-artifact path under test")]
    // SAFETY: the artifact was compiled two statements up by this process's
    // own `omnia::compile`, from a guest the test suite built.
    let trusted = unsafe { Verified::trusted(bytes) };
    runtime
        .admit("full".into(), trusted, Grants::all())
        .await
        .expect("the embedder's own artifact is admitted");
    let sync = call(&runtime, "full", "poke", "trusted").await.expect("sync dispatch");
    assert_eq!(sync, "echoer pong: trusted");
}

// A declared name is bound by its entry alone before the entry ever loads:
// `register` and `admit` are refused under it with nothing seated, and the
// first use that follows still reaches the entry's own bytes.
#[tokio::test]
async fn register_declared() {
    let full = std::fs::read(test_programs::LINK_FULL).expect("reading the full guest");
    let manifest = Manifest::new()
        .guest(GuestEntry::new("full", full.clone()))
        .guest(GuestEntry::new("echoer", test_programs::LINK_ECHOER));
    let runtime = assemble(manifest, |builder| builder).await.expect("deployment boots");

    let err =
        runtime.register("echoer", full.clone(), Grants::all()).await.expect_err("a declared name");
    assert!(format!("{err:#}").contains("declares"), "unexpected error: {err:#}");
    let verified = Verified::wasm(full).expect("the full guest is raw wasm");
    let err =
        runtime.admit("echoer".into(), verified, Grants::all()).await.expect_err("a declared name");
    assert!(format!("{err:#}").contains("declares"), "unexpected error: {err:#}");
    assert!(runtime.registry().get(&GuestId::from("echoer")).is_none(), "nothing was admitted");

    let sync = call(&runtime, "full", "poke", "hi").await.expect("the entry loads at first use");
    assert_eq!(sync, "echoer pong: hi");
}

// Caller-named grants validate against the deployment before the bytes
// load, and a name already active attests on the same digest and grants,
// whatever order the grants list their entries in.
#[tokio::test]
async fn register_grants() {
    let runtime = boot(&[("echoer", test_programs::LINK_ECHOER)]).await.expect("deployment boots");
    let full = std::fs::read(test_programs::LINK_FULL).expect("reading the full guest");

    let err = runtime
        .register("full", full.clone(), Grants::all().mounts(["dta"]))
        .await
        .expect_err("a mount the deployment does not declare");
    assert!(format!("{err:#}").contains("`mounts` entry `dta`"), "unexpected error: {err:#}");
    assert!(runtime.registry().get(&GuestId::from("full")).is_none(), "nothing was admitted");

    let grants = Grants::none().guests(["echoer"]).interfaces(["wasi:*", "omnia-test:*"]);
    runtime.register("full", full.clone(), grants).await.expect("the grants reach every import");
    let reordered = Grants::none().guests(["echoer"]).interfaces(["omnia-test:*", "wasi:*"]);
    runtime.register("full", full.clone(), reordered).await.expect("the same grants attest");
    let err =
        runtime.register("full", full, Grants::all()).await.expect_err("other grants than its own");
    assert!(format!("{err:#}").contains("active under other grants"), "unexpected error: {err:#}");

    let sync = call(&runtime, "full", "poke", "granted").await.expect("sync dispatch");
    assert_eq!(sync, "echoer pong: granted");
}

// Racing registrations of one identity, each pair on tasks of their own so the
// two seats overlap: other bytes conflict, the same bytes and grants attest,
// and the loser of two `admit`s conflicts.
#[tokio::test(flavor = "multi_thread")]
async fn racing_registration() {
    let runtime = boot(&[("echoer", test_programs::LINK_ECHOER)]).await.expect("deployment boots");
    let full = std::fs::read(test_programs::LINK_FULL).expect("reading the full guest");
    let partial = std::fs::read(test_programs::LINK_PARTIAL).expect("reading the partial guest");
    let (full_digest, partial_digest) = (Digest::of(&full), Digest::of(&partial));
    let id = GuestId::from("full");
    let seated = || runtime.registry().get(&id).map(|guest| guest.digest());
    let register = |bytes: Vec<u8>| {
        let runtime = runtime.clone();
        async move { runtime.register("full", bytes, Grants::all()).await }
    };

    // other bytes: one seats, and the other conflicts
    let outcomes = race([full.clone(), partial].map(&register)).await;
    let (winner, lost) = match &outcomes {
        [Ok(()), Err(error)] => (full_digest, error),
        [Err(error), Ok(())] => (partial_digest, error),
        outcomes => panic!("one registration seats and the other conflicts: {outcomes:?}"),
    };
    assert!(format!("{lost:#}").contains("active under other bytes"), "unexpected error: {lost:#}");
    assert_eq!(seated(), Some(winner));

    // the same bytes and grants: both attest one registration
    runtime.deregister(&id).expect("deregistering the late guest");
    let [first, second] = race([full.clone(), full.clone()].map(&register)).await;
    first.expect("the first registration attests");
    second.expect("the second registration attests");
    assert_eq!(seated(), Some(full_digest));

    // two `admit`s: one seats, and the other conflicts
    runtime.deregister(&id).expect("deregistering the late guest");
    let admit = |bytes: Vec<u8>| {
        let (runtime, id) = (runtime.clone(), id.clone());
        let verified = Verified::wasm(bytes).expect("the full guest is raw wasm");
        async move { runtime.admit(id, verified, Grants::all()).await }
    };
    let outcomes = race([full.clone(), full].map(admit)).await;
    let admitted = outcomes.iter().filter(|outcome| outcome.is_ok()).count();
    let conflicts = outcomes
        .iter()
        .filter(|outcome| matches!(outcome, Err(GuestError::Conflict { .. })))
        .count();
    assert_eq!((admitted, conflicts), (1, 1), "{outcomes:?}");
    assert_eq!(seated(), Some(full_digest));
}

// runs both at once, each on a task of its own
async fn race<T: Send + 'static>(
    contenders: [impl Future<Output = T> + Send + 'static; 2],
) -> [T; 2] {
    let [first, second] = contenders.map(tokio::spawn);
    [first.await.expect("the first completes"), second.await.expect("the second completes")]
}

// The relay takes the id `echoer` because `full` hard-codes `ping("echoer",
// ..)` and a call routes on its leading argument; every relay hop then
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
    let text = format!("{err:#}");
    for needle in ["exceeds maximum 3", "MAX_DISPATCH_DEPTH"] {
        assert!(text.contains(needle), "`{needle}` missing from: {text}");
    }
}

// The sleeper takes the id `echoer` for the same reason as the relay. Only
// the caller is awaited, so the test finishes well inside the 2 s the sleeper
// would otherwise hold its store. The sleep goes through `poke-async`, since
// a sync-typed export that blocks traps instead of waiting.
#[tokio::test]
async fn link_sleeper() {
    let runtime = boot_with(
        &[("echoer", test_programs::LINK_SLEEPER), ("full", test_programs::LINK_FULL)],
        |builder| builder.guest_timeout(Duration::from_millis(50)),
    )
    .await
    .expect("deployment boots");

    let err =
        call(&runtime, "full", "poke-async", "sleep").await.expect_err("callee outlives the bound");
    assert!(format!("{err:#}").contains("timed out"), "unexpected error: {err:#}");

    // The target is still reachable after the timed-out call was abandoned.
    let answer = call(&runtime, "full", "poke", "awake").await.expect("dispatch after timeout");
    assert_eq!(answer, "echoer woke: awake");
}

// An unset gate is as wide as the pool, so a hop takes no slot yet can find
// the pool full: here the caller's own instance fills a pool of one.
#[tokio::test]
async fn full_pool() {
    let options = RuntimeOptions {
        pool_max_instances: 1,
        ..RuntimeOptions::default()
    };
    let runtime = boot_with(
        &[("echoer", test_programs::LINK_ECHOER), ("full", test_programs::LINK_FULL)],
        |builder| builder.options(options),
    )
    .await
    .expect("deployment boots");
    assert_eq!(runtime.invocations().limit, 1);

    let err = call(&runtime, "full", "poke", "hi").await.expect_err("the callee finds no instance");
    let text = format!("{err:#}");
    for needle in ["MAX_CONCURRENT_INVOCATIONS", "POOL_MAX_INSTANCES"] {
        assert!(text.contains(needle), "`{needle}` missing from: {text}");
    }
}

// The caller holds the gate's only slot for as long as it runs, so a hop that
// waited for a slot would wait on its own caller.
#[tokio::test]
async fn full_gate() {
    let options = RuntimeOptions {
        max_concurrent_invocations: Some(1),
        ..RuntimeOptions::default()
    };
    let runtime = boot_with(
        &[("echoer", test_programs::LINK_ECHOER), ("full", test_programs::LINK_FULL)],
        |builder| builder.options(options),
    )
    .await
    .expect("deployment boots");

    let poke = tokio::time::timeout(Duration::from_secs(5), call(&runtime, "full", "poke", "hi"));
    let answer = poke.await.expect("the hop waits on no slot").expect("sync dispatch");
    assert_eq!(answer, "echoer pong: hi");
}

// Whichever of the two seats second is refused, in every order, so no call
// crosses the skew. The skewed exporter takes the id `echoer`, the target
// `full` hard-codes, so under path guests `full`'s call is what loads it.
#[tokio::test]
async fn link_skewed() {
    let full = std::fs::read(test_programs::LINK_FULL).expect("reading the full guest");
    let skewed = std::fs::read(test_programs::LINK_SKEWED).expect("reading the skewed guest");
    let seated = |runtime: &Runtime<()>, id: &str| runtime.registry().get(&GuestId::from(id));

    // both at boot
    let Err(err) =
        boot(&[("echoer", test_programs::LINK_SKEWED), ("full", test_programs::LINK_FULL)]).await
    else {
        panic!("skewed exporter booted");
    };
    names_skew(&format!("{err:#}"), ["echoer", "full"]);

    // the exporter after a boot importer
    let runtime = boot(&[("full", test_programs::LINK_FULL)]).await.expect("deployment boots");
    names_skew(&refused_seat(&runtime, "echoer", &skewed).await, ["echoer", "full"]);
    assert!(seated(&runtime, "echoer").is_none(), "nothing was seated");

    // the importer after a boot exporter, twice, so the first refusal left nothing behind
    let runtime = boot(&[("echoer", test_programs::LINK_SKEWED)]).await.expect("deployment boots");
    for _ in 0..2 {
        names_skew(&refused_seat(&runtime, "full", &full).await, ["echoer", "full"]);
    }
    assert!(seated(&runtime, "full").is_none(), "nothing was seated");

    // the exporter after a late importer
    let manifest = Manifest::new().guest(GuestEntry::new("full", test_programs::LINK_FULL));
    let runtime = assemble(manifest, |builder| builder).await.expect("deployment boots");
    runtime.guest(&GuestId::from("full")).await.expect("`full` loads at first use");
    names_skew(&refused_seat(&runtime, "echoer", &skewed).await, ["echoer", "full"]);
    assert!(seated(&runtime, "echoer").is_none(), "nothing was seated");

    // path guests, where the call loads the exporter
    let manifest = Manifest::new()
        .guest(GuestEntry::new("full", test_programs::LINK_FULL))
        .guest(GuestEntry::new("echoer", test_programs::LINK_SKEWED));
    let runtime = assemble(manifest, |builder| builder).await.expect("deployment boots");
    let err = call(&runtime, "full", "poke", "hi").await.expect_err("the exporter is refused");
    names_skew(&format!("{err:#}"), ["echoer", "full"]);
    assert!(seated(&runtime, "echoer").is_none(), "nothing was seated");
}

// admits `bytes` as `id`, which must be refused as invalid
async fn refused_seat(runtime: &Runtime<()>, id: &str, bytes: &[u8]) -> String {
    let verified = Verified::wasm(bytes.to_vec()).expect("the guest is raw wasm");
    match runtime.admit(GuestId::from(id), verified, Grants::all()).await {
        Err(GuestError::Invalid(reason)) => reason,
        outcome => panic!("`{id}` was not refused as invalid: {outcome:?}"),
    }
}

fn names_skew(text: &str, guests: [&str; 2]) {
    let guests = guests.map(|guest| format!("guest `{guest}`"));
    for needle in guests.iter().map(String::as_str).chain(["omnia-test:link/ops", "ping"]) {
        assert!(text.contains(needle), "`{needle}` missing from: {text}");
    }
}

// `full` and the skewed importer agree on `ping`'s parameters but not on its
// result, so whichever of the two seats second is refused, at boot or late.
#[tokio::test]
async fn link_skewed_importer() {
    let Err(err) = boot(&[
        ("full", test_programs::LINK_FULL),
        ("skewed", test_programs::LINK_SKEWED_IMPORTER),
    ])
    .await
    else {
        panic!("two importers that disagree both booted");
    };
    names_skew(&format!("{err:#}"), ["full", "skewed"]);

    let runtime = boot(&[("full", test_programs::LINK_FULL)]).await.expect("deployment boots");
    let skewed = std::fs::read(test_programs::LINK_SKEWED_IMPORTER).expect("reading the importer");
    names_skew(&refused_seat(&runtime, "skewed", &skewed).await, ["full", "skewed"]);
    assert!(runtime.registry().get(&GuestId::from("skewed")).is_none(), "nothing was seated");
}

// `versioned` imports `ops` at 0.1.0 and exports it at 0.1.1, which
// wasmtime's semver-compatible lookup takes for one interface.
#[tokio::test]
async fn link_versioned() {
    let runtime =
        boot(&[("versioned", test_programs::LINK_VERSIONED)]).await.expect("deployment boots");
    let dispatch = |version: &str| {
        runtime.dispatcher().invoke(
            ChainCtx::server(),
            GuestId::from("versioned"),
            Some(format!("omnia-test:versioned/ops@{version}")),
            "ping".into(),
            vec![Val::String("versioned".into()), Val::String("hi".into())],
        )
    };
    let refused = "guest `versioned` exports no interface `omnia-test:versioned/ops@0.1.0`";

    let answer = dispatch("0.1.1").await.expect("the exported name");
    assert_eq!(answer, vec![Val::String("versioned pong: hi".into())]);
    let err = dispatch("0.1.0").await.expect_err("a compatible name is another interface");
    assert!(format!("{err:#}").contains(refused), "unexpected error: {err:#}");

    let err = call(&runtime, "versioned", "poke", "hi").await.expect_err("the import is 0.1.0");
    assert!(format!("{err:#}").contains(refused), "unexpected error: {err:#}");
}

// `take` nests its resource in an optional record, so the refusal must walk
// the signature's types, not just its parameters; it fires as the importer
// is wired, before any call.
#[tokio::test]
async fn link_handle() {
    let Err(err) = boot(&[("handle", test_programs::LINK_HANDLE)]).await else {
        panic!("a resource-carrying import was wired");
    };

    let text = format!("{err:#}");
    for needle in ["`handle`", "omnia-test:link/tokens", "take", "carries a resource"] {
        assert!(text.contains(needle), "`{needle}` missing from: {text}");
    }
}

// Unlike `take`, `count` is a plain signature, so the importer boots and its
// call is what is refused, before any target is resolved.
#[tokio::test]
async fn link_anonymous() {
    let runtime =
        boot(&[("anonymous", test_programs::LINK_ANONYMOUS)]).await.expect("deployment boots");

    let err = call(&runtime, "anonymous", "poke", "hi").await.expect_err("`count` names no guest");
    let text = format!("{err:#}");
    for needle in ["omnia-test:link/counter", "count", "leading string identity"] {
        assert!(text.contains(needle), "`{needle}` missing from: {text}");
    }
}

// A command deployment's arguments reach the command run alone. No command
// runs here: a host dispatch builds its store through the store factory, and
// a lease is how every trigger builds one.
#[tokio::test]
async fn link_arguments() {
    let runtime = boot_with(&[("arguments", test_programs::LINK_ARGUMENTS)], |builder| {
        builder.mode(Mode::Command).args(vec!["--secret".to_owned()])
    })
    .await
    .expect("deployment boots");
    let params = [Val::String("arguments".into()), Val::String("hi".into())];
    let unseen = vec![Val::String("arguments argv: []".into())];

    let answer = runtime
        .dispatcher()
        .invoke(
            ChainCtx::server(),
            GuestId::from("arguments"),
            Some("omnia-test:link/ops".into()),
            "ping".into(),
            params.to_vec(),
        )
        .await
        .expect("host dispatch");
    assert_eq!(answer, unseen);

    let entry = runtime.guest(&GuestId::from("arguments")).await.expect("resolving the guest");
    let results = runtime
        .lease(entry)
        .await
        .invoke(async |mut entered, instance, _bound| {
            let ops = instance
                .get_export_index(&mut entered, None, "omnia-test:link/ops")
                .expect("the guest exports `ops`");
            let ping = instance
                .get_export_index(&mut entered, Some(&ops), "ping")
                .expect("`ops` has `ping`");
            let ping = instance.get_func(&mut entered, ping).expect("`ping` is a function");
            let mut results = vec![Val::Bool(false)];
            ping.call_async(&mut entered, &params, &mut results).await.expect("calling `ping`");
            results
        })
        .await
        .expect("entering the guest");
    assert_eq!(results, unseen);
}

// The manifest is validated before any guest loads, so no row's source need
// exist. Two files of one stem collide on the name each derives.
#[tokio::test]
async fn invalid_manifests() {
    let stems = Path::new(env!("CARGO_TARGET_TMPDIR")).join("duplicate_stem.toml");
    std::fs::write(
        &stems,
        "[[guest]]\nsource.path = \"./a/echo.wasm\"\n\n\
         [[guest]]\nsource.path = \"./b/echo.wasm\"\n",
    )
    .expect("writing the manifest");
    let mut unix = Manifest::new().guest(GuestEntry::new("only", "./only.wasm"));
    unix.transport = Transport {
        default: TransportKind::Unix,
    };

    let rows = [
        (unix, "transport `Unix` is not yet implemented"),
        (
            Manifest::new()
                .guest(GuestEntry::new("a", "./a.wasm").command())
                .guest(GuestEntry::new("b", "./b.wasm").command()),
            "at most one guest may be the command guest",
        ),
        (
            Manifest::new()
                .guest(GuestEntry::new("same", "./a.wasm"))
                .guest(GuestEntry::new("same", "./b.wasm")),
            "duplicate [[guest]] name `same`",
        ),
        (Manifest::load(&stems).expect("the manifest parses"), "duplicate [[guest]] name `echo`"),
        (Manifest::new().guest(GuestEntry::new("", b"\0asm")), "names no guest"),
        (Manifest::new(), "defines no [[guest]] entry"),
    ];
    for (manifest, needle) in rows {
        let Err(err) = assemble(manifest, |builder| builder).await else {
            panic!("a manifest refused for `{needle}` was built");
        };
        assert!(format!("{err:#}").contains(needle), "unexpected error: {err:#}");
    }

    assemble(Manifest::new(), omnia::DeploymentBuilder::dynamic)
        .await
        .expect("a dynamic deployment may start with no guests");
}

// The builder's override is validated with the options it lands on, so a
// zero set in code fails the build as `GUEST_TIMEOUT_MS=0` fails a load.
#[tokio::test]
async fn zero_guest_timeout() {
    let Err(err) =
        assemble(Manifest::new(), |builder| builder.dynamic().guest_timeout(Duration::ZERO)).await
    else {
        panic!("a zero guest timeout was accepted");
    };
    assert!(format!("{err:#}").contains("GUEST_TIMEOUT_MS"), "unexpected error: {err:#}");
}

// The refusal counts the arguments and never echoes one, since an argument
// can hold a secret and a server's stderr lands in its logs.
#[tokio::test]
async fn server_arguments() {
    let Err(err) =
        assemble(Manifest::new(), |builder| builder.dynamic().args(vec!["stray".to_owned()])).await
    else {
        panic!("a server took arguments");
    };
    let text = format!("{err:#}");
    assert!(text.contains("takes no arguments: 1 given"), "unexpected error: {text}");
    assert!(!text.contains("stray"), "the refusal echoes an argument: {text}");
}

// A command root's wall-clock policy is carried to every hop beneath it.
#[test]
fn command_chain() {
    let policy = ChainPolicy {
        max_depth: 2,
        timeout: Duration::from_secs(1),
    };
    let callee = GuestId::from("callee");
    let root = ChainCtx::command();

    let child = policy.enter(&root, &callee).expect("within depth");
    assert!(child.uncapped);
    assert_eq!(child.depth, 1);

    let grandchild = policy.enter(&child, &callee).expect("within depth");
    assert!(grandchild.uncapped);
    assert_eq!(grandchild.depth, 2);
}

#[test]
fn runtime_options() {
    fn load(vars: &[(&str, &str)]) -> Result<RuntimeOptions> {
        RuntimeOptions::load_from(|name| {
            vars.iter().find(|(key, _)| *key == name).map(|(_, value)| (*value).to_owned())
        })
    }

    // `Enabled` spellings through `POOL_PAGEMAP_SCAN`
    for (spelling, expected) in
        [("auto", Enabled::Auto), ("YES", Enabled::Yes), (" no ", Enabled::No)]
    {
        let options = load(&[("POOL_PAGEMAP_SCAN", spelling)]).expect("valid spelling loads");
        assert_eq!(options.pool_pagemap_scan, expected);
    }
    let err = load(&[("POOL_PAGEMAP_SCAN", "maybe")]).expect_err("invalid spelling is refused");
    assert!(
        format!("{err:#}").contains("expected one of `auto`/`yes`/`no`"),
        "unexpected error: {err:#}"
    );
}

// The relay forwards to whichever guest its first argument names, so one
// program reaches a granted target and an ungranted one. The ungranted target
// is declared by path, so a refusal that came after its first use would load
// it.
#[tokio::test]
async fn caller_grants() {
    let relay = std::fs::read(test_programs::LINK_RELAY).expect("reading the relay");
    let echoer = std::fs::read(test_programs::LINK_ECHOER).expect("reading the echoer");
    let grants = Grants::none().guests(["echoer"]).interfaces(["wasi:*", "omnia-test:*"]);
    let manifest = Manifest::new()
        .guest(GuestEntry::new("relay", relay).grants(grants))
        .guest(GuestEntry::new("echoer", echoer))
        .guest(GuestEntry::new("other", test_programs::LINK_ECHOER));
    let runtime = assemble(manifest, |builder| builder).await.expect("deployment boots");

    let relay_to = |target: &str| {
        runtime.dispatcher().invoke(
            ChainCtx::server(),
            GuestId::from("relay"),
            Some("omnia-test:link/ops".into()),
            "ping".into(),
            vec![Val::String(target.to_owned()), Val::String("1".into())],
        )
    };

    let answer = relay_to("echoer").await.expect("the relay's grants reach `echoer`");
    assert_eq!(answer, vec![Val::String("echoer pong: 0".into())]);

    let err = relay_to("other").await.expect_err("the relay's grants reach no `other`");
    assert!(
        format!("{err:#}").contains("guest `relay` may not call `other`"),
        "unexpected error: {err:#}"
    );
    assert!(runtime.registry().get(&GuestId::from("other")).is_none(), "nothing was loaded");
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

// `echoer` is declared by path, so a hop refused for depth after its first use
// would load it. The relay runs at the bound, so its own hop is over it.
#[tokio::test]
async fn over_bound_hop() {
    let relay = std::fs::read(test_programs::LINK_RELAY).expect("reading the relay");
    let manifest = Manifest::new()
        .guest(GuestEntry::new("relay", relay))
        .guest(GuestEntry::new("echoer", test_programs::LINK_ECHOER));
    let runtime = assemble(manifest, |builder| builder.max_dispatch_depth(1))
        .await
        .expect("deployment boots");
    let echoer = GuestId::from("echoer");
    let dispatch = |caller: ChainCtx, target: &str, message: &str| {
        runtime.dispatcher().invoke(
            caller,
            GuestId::from(target),
            Some("omnia-test:link/ops".into()),
            "ping".into(),
            vec![Val::String("echoer".into()), Val::String(message.to_owned())],
        )
    };

    let err = dispatch(ChainCtx::server(), "relay", "1").await.expect_err("the relay's hop");
    assert!(format!("{err:#}").contains("exceeds maximum 1"), "unexpected error: {err:#}");
    assert!(runtime.registry().get(&echoer).is_none(), "the link call loaded its target");

    let deep = ChainCtx {
        depth: 1,
        ..ChainCtx::server()
    };
    let err = dispatch(deep, "echoer", "hi").await.expect_err("a host call from the bound");
    assert!(format!("{err:#}").contains("exceeds maximum 1"), "unexpected error: {err:#}");
    assert!(runtime.registry().get(&echoer).is_none(), "the host call loaded its target");
}

// A call through the runtime's dispatcher is a root invocation, so it waits on
// a full gate as a trigger does rather than running beside the slot's holder.
#[tokio::test]
async fn queued_dispatch() {
    let options = RuntimeOptions {
        max_concurrent_invocations: Some(1),
        ..RuntimeOptions::default()
    };
    let runtime =
        boot_with(&[("echoer", test_programs::LINK_ECHOER)], |builder| builder.options(options))
            .await
            .expect("deployment boots");

    // with the only slot held, the dispatch waits for it
    let echoer = runtime.guest(&GuestId::from("echoer")).await.expect("resolving the guest");
    let lease = runtime.try_lease(echoer, "test").expect("the gate is empty");
    let release = async {
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert_eq!(runtime.invocations().waiting, 1);
        drop(lease);
    };
    let dispatch = runtime.dispatcher().invoke(
        ChainCtx::server(),
        GuestId::from("echoer"),
        Some("omnia-test:link/ops".into()),
        "ping".into(),
        vec![Val::String("echoer".into()), Val::String("hi".into())],
    );
    let ((), answer) = tokio::join!(release, dispatch);
    assert_eq!(answer.expect("host dispatch"), vec![Val::String("echoer pong: hi".into())]);
    assert_eq!(runtime.invocations().in_flight, 0);
}

// Nothing calls `shutdown`, so once a link call has run the runtime must hold
// no handle to itself.
#[tokio::test]
async fn dropped_runtime() {
    let runtime =
        boot(&[("echoer", test_programs::LINK_ECHOER), ("full", test_programs::LINK_FULL)])
            .await
            .expect("deployment boots");
    let answer = call(&runtime, "full", "poke", "hi").await.expect("sync dispatch");
    assert_eq!(answer, "echoer pong: hi");

    let weak = runtime.downgrade();
    drop(runtime);
    assert!(weak.upgrade().is_none(), "the runtime outlived its last handle");
}
