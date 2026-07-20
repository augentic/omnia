//! Host-mediated dynamic linking seam.
//!
//! Builds the `examples/guest-link` deployment — `router` imports
//! `omnia:link/echo`, `responder` exports it — wires the serve side, and
//! drives `router.run`. It proves the end-to-end dispatch: the call routes
//! through the runtime core's selector to the responder over the in-process
//! wRPC carrier, the responder is instantiated fresh per call
//! (instance-per-call), and the typed result returns to the caller. Two calls
//! confirm the multi-use carrier (a fresh frame connection per call).
//!
//! Each test builds its own runtime (cheap with serialized guests) so the
//! clone-counting witness is not disturbed by concurrent tests.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use anyhow::{Context as _, Result, bail, ensure};
use omnia::wasmtime::component::Val;
use omnia::{
    DeploymentBuilder, GuestArtifact, GuestEntry, GuestId, Manifest, MountRegistry, Runtime,
    serve_links,
};
use omnia_testkit::find_guest;

use crate::fixture;

/// Per-store context: the library [`omnia::StoreCtx`] over the counting
/// [`Counter`] bundle. No host backend — the link path needs only the WASI and
/// wRPC views, which `StoreCtx` supplies from its `base`.
type TestCtx = omnia::StoreCtx<Counter>;

/// A backend-less bundle whose [`Clone`] bumps a shared counter.
///
/// The library [`Runtime::store`] clones the bundle to build each per-guest
/// store, so a fixed, nonzero amount of bundle cloning happens per store built
/// (the caller and every freshly dispatched callee). Equal nonzero clone deltas
/// across calls therefore witness instance-per-call: a cached/reused callee would
/// build fewer stores — and clone the bundle fewer times — on a later call.
#[derive(Default)]
struct Counter {
    clones: Arc<AtomicUsize>,
}

impl Clone for Counter {
    fn clone(&self) -> Self {
        self.clones.fetch_add(1, Ordering::SeqCst);
        Self {
            clones: Arc::clone(&self.clones),
        }
    }
}

/// Instantiate the router fresh, call its `export` with `message`, and return
/// the echoed string.
async fn call_router(runtime: &Runtime<Counter>, export: &str, message: &str) -> Result<String> {
    let guest =
        runtime.registry().get(&GuestId::from("router")).context("router guest is registered")?;
    let mut store = runtime.build_store(runtime.store());
    let instance = runtime
        .instantiate(guest.instance_pre(), &mut store)
        .await
        .context("instantiating router")?;
    let run = instance
        .get_func(&mut store, export)
        .with_context(|| format!("router exports `{export}`"))?;

    let mut results = vec![Val::Bool(false)];
    run.call_async(&mut store, &[Val::String(message.to_owned())], &mut results)
        .await
        .map_err(anyhow::Error::from)
        .with_context(|| format!("calling router.{export}"))?;

    match results.into_iter().next() {
        Some(Val::String(echoed)) => Ok(echoed),
        other => bail!("router.{export} returned a non-string result: {other:?}"),
    }
}

/// Instantiate the router fresh and call `export(target, message)`.
async fn call_router_export_to(
    runtime: &Runtime<Counter>, export: &str, target: &str, message: &str,
) -> Result<String> {
    let guest =
        runtime.registry().get(&GuestId::from("router")).context("router guest is registered")?;
    let mut store = runtime.build_store(runtime.store());
    let instance = runtime
        .instantiate(guest.instance_pre(), &mut store)
        .await
        .context("instantiating router")?;
    let run_to = instance
        .get_func(&mut store, export)
        .with_context(|| format!("router exports `{export}`"))?;

    let mut results = vec![Val::Bool(false)];
    run_to
        .call_async(
            &mut store,
            &[Val::String(target.to_owned()), Val::String(message.to_owned())],
            &mut results,
        )
        .await
        .map_err(anyhow::Error::from)
        .with_context(|| format!("calling router.{export}"))?;

    match results.into_iter().next() {
        Some(Val::String(echoed)) => Ok(echoed),
        other => bail!("router.{export} returned a non-string result: {other:?}"),
    }
}

/// Call `run-to(target, message)` — the arbitrary-target leg that reaches
/// dynamically registered guests.
async fn call_router_to(runtime: &Runtime<Counter>, target: &str, message: &str) -> Result<String> {
    call_router_export_to(runtime, "run-to", target, message).await
}

/// Call `run-to-slow(target, message)` — the async-lifted arbitrary-target
/// leg whose callee parks on a timer before answering.
async fn call_router_to_slow(
    runtime: &Runtime<Counter>, target: &str, message: &str,
) -> Result<String> {
    call_router_export_to(runtime, "run-to-slow", target, message).await
}

/// Locate the serialized `.bin` for `file` and wrap it as a pre-compiled
/// registration artifact. Fails fast (rather than substituting raw wasm) when
/// the `.bin` is missing, so the pre-compiled path is genuinely exercised.
fn precompiled(file: &str) -> Result<GuestArtifact> {
    let path = find_guest(file);
    ensure!(
        path.extension().is_some_and(|ext| ext == "bin"),
        "{} has no serialized .bin sibling; run `cargo make test-guests`",
        path.display()
    );
    let bytes =
        std::fs::read(&path).with_context(|| format!("reading guest {}", path.display()))?;
    // SAFETY: the artifact was built and serialized by this workspace's own
    // `cargo make test-guests` pipeline (omnia's compile path).
    Ok(unsafe { GuestArtifact::precompiled(bytes) })
}

/// The raw-wasm dual of [`precompiled`], exercising the safe JIT constructor.
/// Names the `.wasm` sibling explicitly so the pre-compiled `.bin` can never
/// silently substitute.
fn raw_wasm(file: &str) -> Result<GuestArtifact> {
    let path = find_guest(file).with_extension("wasm");
    let bytes =
        std::fs::read(&path).with_context(|| format!("reading guest {}", path.display()))?;
    Ok(GuestArtifact::wasm(bytes))
}

/// Build the two-guest deployment and wire the serve side, returning the
/// runtime plus the shared bundle-clone counter.
async fn build_runtime() -> Result<(Runtime<Counter>, Arc<AtomicUsize>)> {
    let responder = find_guest("guest_link_responder_wasm.wasm");
    let router = find_guest("guest_link_router_wasm.wasm");

    let manifest = Manifest::new()
        .guest(GuestEntry::new("responder", responder))
        .guest(GuestEntry::new("router", router).link("omnia:link/echo"));

    let builder = DeploymentBuilder::new().manifest(manifest).precompiled();
    // SAFETY: `find_guest` only returns artifacts this workspace built and
    // serialized itself (`cargo make test-guests`).
    let deployment = unsafe { builder.build::<TestCtx>() }.await.context("building runtime")?;
    let registry = deployment.into_registry().context("assembling registry")?;
    let clones = Arc::new(AtomicUsize::new(0));
    let runtime = Runtime::<Counter>::from_parts(
        Arc::new(registry),
        Vec::new(),
        Arc::new(MountRegistry::default()),
        Counter {
            clones: Arc::clone(&clones),
        },
    );

    // Wire the serve side of `omnia:link/echo` (responder) and bind the
    // in-process carrier — the work `Runtime::new` does for a real deployment
    // (`from_parts` is the low-level constructor and leaves it to the caller).
    serve_links(&runtime).await.context("wiring link serve side")?;
    Ok((runtime, clones))
}

// The router guest calls the responder over a host-mediated link, proving
// dispatch and instance-per-call.
#[test]
fn dispatch() -> Result<()> {
    fixture::RT.block_on(async {
        let (runtime, clones) = build_runtime().await?;

        // Two calls prove the multi-use carrier (a fresh frame connection per call)
        // and instance-per-call: each dispatch instantiates the responder fresh on a
        // new store. The bundle clone count rises by a fixed, nonzero amount per call
        // (router caller store + responder callee store); equal deltas across the two
        // calls witness that the second call rebuilds the callee rather than reusing
        // a cached one.
        let mut per_call: Option<usize> = None;
        for message in ["hello", "world"] {
            let before = clones.load(Ordering::SeqCst);
            let echoed = call_router(&runtime, "run", message).await?;
            let delta = clones.load(Ordering::SeqCst) - before;

            assert_eq!(echoed, format!("responder echoes: {message}"));
            assert!(delta > 0, "each call builds at least one store");
            match per_call {
                None => per_call = Some(delta),
                Some(expected) => {
                    assert_eq!(
                        delta, expected,
                        "each call does identical work (instance-per-call)"
                    );
                }
            }
        }

        Ok(())
    })
}

// The async-typed leg: `run-slow` is an async-lifted export calling the
// async-typed `echo-slow` import through the `func_new_concurrent` polyfill,
// and the responder parks on a host timer before answering — the dispatch
// round-trip completes against a genuinely pending callee.
#[test]
fn dispatch_async() -> Result<()> {
    fixture::RT.block_on(async {
        let (runtime, _clones) = build_runtime().await?;

        // Two calls again prove the multi-use carrier under the concurrent path.
        for message in ["hello", "world"] {
            let echoed = call_router(&runtime, "run-slow", message).await?;
            assert_eq!(echoed, format!("responder echoes slowly: {message}"));
        }

        Ok(())
    })
}

// A guest registered after startup (absent from the manifest) is reachable via
// host-mediated link dispatch — serve-at-register — and via host→guest
// dispatch, while static dispatch is undisturbed. Registration loads the
// serialized artifact (the unsafe `precompiled` constructor).
#[test]
fn register_then_dispatch() -> Result<()> {
    fixture::RT.block_on(async {
        let (runtime, _clones) = build_runtime().await?;
        runtime.register("extra", precompiled("guest_link_extra_wasm.wasm")?).await?;

        // Guest→guest: the static router names the registered guest.
        let echoed = call_router_to(&runtime, "extra", "hello").await?;
        assert_eq!(echoed, "extra echoes from extra: hello");

        // Host→guest: the dispatcher reaches it like any static guest.
        let results = runtime
            .dispatcher()
            .invoke(
                GuestId::from("extra"),
                None,
                "echo".to_owned(),
                vec![Val::String("extra".to_owned()), Val::String("hi".to_owned())],
            )
            .await?;
        assert_eq!(results, vec![Val::String("extra echoes from extra: hi".to_owned())]);

        // Static dispatch is undisturbed.
        let echoed = call_router_to(&runtime, "responder", "hello").await?;
        assert_eq!(echoed, "responder echoes: hello");

        Ok(())
    })
}

// Deregistration makes new dispatches fail as unregistered on both dispatch
// paths; the static guests are unaffected.
#[test]
fn deregister_then_dispatch() -> Result<()> {
    fixture::RT.block_on(async {
        let (runtime, _clones) = build_runtime().await?;
        runtime.register("extra", precompiled("guest_link_extra_wasm.wasm")?).await?;
        call_router_to(&runtime, "extra", "hello").await?;

        runtime.deregister(&GuestId::from("extra"))?;

        call_router_to(&runtime, "extra", "hello")
            .await
            .expect_err("link dispatch to a deregistered guest must fail");
        runtime
            .dispatcher()
            .invoke(GuestId::from("extra"), None, "echo".to_owned(), Vec::new())
            .await
            .expect_err("host dispatch to a deregistered guest must fail");

        let echoed = call_router_to(&runtime, "responder", "hello").await?;
        assert_eq!(echoed, "responder echoes: hello");

        Ok(())
    })
}

// Deregister + re-register with different bytes swaps the guest's behavior —
// the upgrade story. The first leg registers the responder's bytes under the
// dynamic id (raw wasm, the safe `wasm` constructor); the second swaps in the
// extra guest's bytes.
#[test]
fn upgrade_swap() -> Result<()> {
    fixture::RT.block_on(async {
        let (runtime, _clones) = build_runtime().await?;

        runtime.register("extra", raw_wasm("guest_link_responder_wasm.wasm")?).await?;
        let echoed = call_router_to(&runtime, "extra", "hello").await?;
        assert_eq!(echoed, "extra echoes: hello", "first registration answers");

        runtime.deregister(&GuestId::from("extra"))?;
        runtime.register("extra", precompiled("guest_link_extra_wasm.wasm")?).await?;
        let echoed = call_router_to(&runtime, "extra", "hello").await?;
        assert_eq!(echoed, "extra echoes from extra: hello", "swapped bytes answer");

        Ok(())
    })
}

// Static entries win: a static id can be neither shadowed by registration nor
// deregistered; an unknown id cannot be deregistered.
#[test]
fn static_ids_protected() -> Result<()> {
    fixture::RT.block_on(async {
        let (runtime, _clones) = build_runtime().await?;

        let error = runtime
            .register("router", precompiled("guest_link_extra_wasm.wasm")?)
            .await
            .expect_err("registering over a static id must fail");
        assert!(error.to_string().contains("already registered"), "{error}");

        let error = runtime
            .deregister(&GuestId::from("router"))
            .expect_err("deregistering a static entry must fail");
        assert!(error.to_string().contains("static"), "{error}");

        runtime
            .deregister(&GuestId::from("ghost"))
            .expect_err("deregistering an unknown id must fail");

        Ok(())
    })
}

// A failed registration (imports outside the linked host set) leaves no
// partial state: the id stays unregistered and remains usable.
#[test]
fn register_failure_no_partial_state() -> Result<()> {
    fixture::RT.block_on(async {
        let (runtime, _clones) = build_runtime().await?;

        // The conformance guest imports host interfaces (keyvalue, blobstore,
        // ...) this deployment never linked, so pre-instantiation fails.
        runtime
            .register("extra", precompiled("conformance_wasm.wasm")?)
            .await
            .expect_err("a guest with unsatisfied imports must fail registration");
        assert!(
            runtime.registry().get(&GuestId::from("extra")).is_none(),
            "a failed registration must not publish the guest"
        );

        // The id is fully reusable: a valid registration under it succeeds.
        runtime.register("extra", precompiled("guest_link_extra_wasm.wasm")?).await?;
        let echoed = call_router_to(&runtime, "extra", "hello").await?;
        assert_eq!(echoed, "extra echoes from extra: hello");

        Ok(())
    })
}

// Two concurrent registrations of one id: publication is transactional, so
// exactly one wins, the winner is callable, and the loser leaves no partial
// state behind (the id deregisters cleanly exactly once).
#[test]
fn register_concurrent_same_id() -> Result<()> {
    fixture::RT.block_on(async {
        let (runtime, _clones) = build_runtime().await?;

        let first = {
            let runtime = runtime.clone();
            let artifact = precompiled("guest_link_extra_wasm.wasm")?;
            tokio::spawn(async move { runtime.register("extra", artifact).await })
        };
        let second = {
            let runtime = runtime.clone();
            let artifact = precompiled("guest_link_extra_wasm.wasm")?;
            tokio::spawn(async move { runtime.register("extra", artifact).await })
        };
        let outcomes = [first.await.expect("register task"), second.await.expect("register task")];
        let wins = outcomes.iter().filter(|outcome| outcome.is_ok()).count();
        assert_eq!(wins, 1, "exactly one concurrent registration wins: {outcomes:?}");

        // The winner is fully published: reachable via link dispatch, and its
        // registry entry deregisters exactly once.
        let echoed = call_router_to(&runtime, "extra", "hello").await?;
        assert_eq!(echoed, "extra echoes from extra: hello");
        runtime.deregister(&GuestId::from("extra"))?;
        runtime
            .deregister(&GuestId::from("extra"))
            .expect_err("the loser must not have left a second entry behind");

        Ok(())
    })
}

// Concurrent register/deregister churn on two ids: after every successful
// registration the registry and link dispatch agree the guest is reachable,
// and after every deregistration they agree it is gone, while static dispatch
// stays undisturbed throughout.
#[test]
fn lifecycle_churn_agrees() -> Result<()> {
    fixture::RT.block_on(async {
        let (runtime, _clones) = build_runtime().await?;

        let mut churners = Vec::new();
        for id in ["extra-a", "extra-b"] {
            let runtime = runtime.clone();
            churners.push(tokio::spawn(async move {
                for _ in 0..5 {
                    runtime.register(id, precompiled("guest_link_extra_wasm.wasm")?).await?;
                    ensure!(
                        runtime.registry().get(&GuestId::from(id)).is_some(),
                        "`{id}` is in the registry after registration"
                    );
                    let echoed = call_router_to(&runtime, id, "hello").await?;
                    ensure!(
                        echoed == format!("{id} echoes from extra: hello"),
                        "`{id}` is link-dispatchable after registration: {echoed}"
                    );

                    runtime.deregister(&GuestId::from(id))?;
                    ensure!(
                        runtime.registry().get(&GuestId::from(id)).is_none(),
                        "`{id}` left the registry after deregistration"
                    );
                    ensure!(
                        call_router_to(&runtime, id, "hello").await.is_err(),
                        "`{id}` is unreachable after deregistration"
                    );
                }
                anyhow::Ok(())
            }));
        }
        let hammer = {
            let runtime = runtime.clone();
            tokio::spawn(async move {
                for _ in 0..10 {
                    let echoed = call_router_to(&runtime, "responder", "hello").await?;
                    ensure!(echoed == "responder echoes: hello", "static dispatch is stable");
                }
                anyhow::Ok(())
            })
        };

        for churner in churners {
            churner.await.expect("churn task")?;
        }
        hammer.await.expect("static dispatch task")?;

        Ok(())
    })
}

// A slow invocation that starts before deregistration completes afterward:
// in-flight calls hold their own instance and server handles, so removal only
// stops *new* dispatches.
#[test]
fn deregister_in_flight_completes() -> Result<()> {
    fixture::RT.block_on(async {
        let (runtime, clones) = build_runtime().await?;
        runtime.register("extra", precompiled("guest_link_extra_wasm.wasm")?).await?;

        // Measure the bundle-clone cost of one complete call (caller store +
        // callee store): once a later call's delta reaches it, the callee's
        // store exists, so the invocation was accepted by the serve side.
        let before = clones.load(Ordering::SeqCst);
        call_router_to(&runtime, "extra", "probe").await?;
        let per_call = clones.load(Ordering::SeqCst) - before;
        assert!(per_call > 0, "a call clones the bundle");

        // Start the slow call, then wait until it is genuinely inside the
        // callee (its clone delta reached a full call's) before deregistering.
        let baseline = clones.load(Ordering::SeqCst);
        let in_flight = {
            let runtime = runtime.clone();
            tokio::spawn(async move { call_router_to_slow(&runtime, "extra", "hello").await })
        };
        while clones.load(Ordering::SeqCst) < baseline + per_call {
            tokio::time::sleep(Duration::from_millis(1)).await;
        }

        runtime.deregister(&GuestId::from("extra"))?;

        // New dispatches fail immediately...
        call_router_to(&runtime, "extra", "again")
            .await
            .expect_err("a new dispatch after deregistration must fail");
        // ...while the pending invocation completes on the handles it holds.
        let echoed = in_flight.await.expect("in-flight call task")?;
        assert_eq!(echoed, "extra echoes slowly from extra: hello");

        Ok(())
    })
}

// Bootstrap wires no import polyfill here (the only static guest, the
// responder, *exports* `echo` but imports nothing), so a dynamically
// registered router proves `polyfill_late` wires the host-mediated import
// from the late component's own types — both the sync- and async-typed legs.
#[test]
fn late_import_polyfilled() -> Result<()> {
    fixture::RT.block_on(async {
        let responder = find_guest("guest_link_responder_wasm.wasm");
        let manifest =
            Manifest::new().guest(GuestEntry::new("responder", responder).link("omnia:link/echo"));

        let builder = DeploymentBuilder::new().manifest(manifest).precompiled();
        // SAFETY: `find_guest` only returns artifacts this workspace built and
        // serialized itself (`cargo make test-guests`).
        let deployment = unsafe { builder.build::<TestCtx>() }.await.context("building runtime")?;
        let registry = deployment.into_registry().context("assembling registry")?;
        let runtime = Runtime::<Counter>::from_parts(
            Arc::new(registry),
            Vec::new(),
            Arc::new(MountRegistry::default()),
            Counter::default(),
        );
        serve_links(&runtime).await.context("wiring link serve side")?;

        // The only guest importing `omnia:link/echo` arrives after bootstrap.
        runtime.register("router", precompiled("guest_link_router_wasm.wasm")?).await?;

        let echoed = call_router(&runtime, "run", "hello").await?;
        assert_eq!(echoed, "responder echoes: hello", "late sync-typed import dispatches");
        let echoed = call_router(&runtime, "run-slow", "hello").await?;
        assert_eq!(echoed, "responder echoes slowly: hello", "late async-typed import dispatches");

        Ok(())
    })
}

// A `dynamic()` deployment starts with zero static guests and is populated
// entirely at run time; host→guest dispatch reaches the registered guest.
#[test]
fn dynamic_empty_deployment() -> Result<()> {
    fixture::RT.block_on(async {
        let deployment = DeploymentBuilder::new()
            .dynamic()
            .build::<TestCtx>()
            .await
            .context("building empty dynamic deployment")?;
        let registry = deployment.into_registry().context("assembling registry")?;
        let runtime = Runtime::<Counter>::from_parts(
            Arc::new(registry),
            Vec::new(),
            Arc::new(MountRegistry::default()),
            Counter::default(),
        );
        assert!(runtime.registry().is_empty(), "a dynamic deployment starts empty");

        // No `link` union is declared, so there is no serve side to wire;
        // host→guest dispatch needs no transport.
        runtime.register("extra", precompiled("guest_link_extra_wasm.wasm")?).await?;
        let results = runtime
            .dispatcher()
            .invoke(
                GuestId::from("extra"),
                None,
                "echo".to_owned(),
                vec![Val::String("extra".to_owned()), Val::String("hi".to_owned())],
            )
            .await?;
        assert_eq!(results, vec![Val::String("extra echoes from extra: hi".to_owned())]);

        Ok(())
    })
}
