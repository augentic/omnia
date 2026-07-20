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

use anyhow::{Context as _, Result, bail};
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

/// Instantiate the router fresh and call `run-to(target, message)` — the
/// arbitrary-target leg that reaches dynamically registered guests.
async fn call_router_to(runtime: &Runtime<Counter>, target: &str, message: &str) -> Result<String> {
    let guest =
        runtime.registry().get(&GuestId::from("router")).context("router guest is registered")?;
    let mut store = runtime.build_store(runtime.store());
    let instance = runtime
        .instantiate(guest.instance_pre(), &mut store)
        .await
        .context("instantiating router")?;
    let run_to = instance.get_func(&mut store, "run-to").context("router exports `run-to`")?;

    let mut results = vec![Val::Bool(false)];
    run_to
        .call_async(
            &mut store,
            &[Val::String(target.to_owned()), Val::String(message.to_owned())],
            &mut results,
        )
        .await
        .map_err(anyhow::Error::from)
        .context("calling router.run-to")?;

    match results.into_iter().next() {
        Some(Val::String(echoed)) => Ok(echoed),
        other => bail!("router.run-to returned a non-string result: {other:?}"),
    }
}

/// Locate a pre-built guest and wrap it as a registration artifact:
/// `Precompiled` for a serialized `.bin`, `Wasm` for raw wasm.
fn artifact(file: &str) -> Result<GuestArtifact> {
    let path = find_guest(file);
    let bytes =
        std::fs::read(&path).with_context(|| format!("reading guest {}", path.display()))?;
    Ok(if path.extension().is_some_and(|ext| ext == "bin") {
        GuestArtifact::Precompiled(bytes)
    } else {
        GuestArtifact::Wasm(bytes)
    })
}

/// The raw-wasm dual of [`artifact`], exercising `GuestArtifact::Wasm`.
fn raw_wasm(file: &str) -> Result<GuestArtifact> {
    let path = find_guest(file).with_extension("wasm");
    let bytes =
        std::fs::read(&path).with_context(|| format!("reading guest {}", path.display()))?;
    Ok(GuestArtifact::Wasm(bytes))
}

/// Build the two-guest deployment and wire the serve side, returning the
/// runtime plus the shared bundle-clone counter.
async fn build_runtime() -> Result<(Runtime<Counter>, Arc<AtomicUsize>)> {
    let responder = find_guest("guest_link_responder_wasm.wasm");
    let router = find_guest("guest_link_router_wasm.wasm");

    let manifest = Manifest::new()
        .guest(GuestEntry::new("responder", responder))
        .guest(GuestEntry::new("router", router).link("omnia:link/echo"));

    let deployment = DeploymentBuilder::new()
        .manifest(manifest)
        .build::<TestCtx>()
        .await
        .context("building runtime")?;
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
    // in-process carrier — the work the generated `start()` does for a real
    // deployment.
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
// serialized artifact (the `Precompiled` path).
#[test]
fn register_then_dispatch() -> Result<()> {
    fixture::RT.block_on(async {
        let (runtime, _clones) = build_runtime().await?;
        runtime.register("extra", artifact("guest_link_extra_wasm.wasm")?).await?;

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
        runtime.register("extra", artifact("guest_link_extra_wasm.wasm")?).await?;
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
// dynamic id (raw wasm, the `Wasm` artifact path); the second swaps in the
// extra guest's bytes.
#[test]
fn upgrade_swap() -> Result<()> {
    fixture::RT.block_on(async {
        let (runtime, _clones) = build_runtime().await?;

        runtime.register("extra", raw_wasm("guest_link_responder_wasm.wasm")?).await?;
        let echoed = call_router_to(&runtime, "extra", "hello").await?;
        assert_eq!(echoed, "extra echoes: hello", "first registration answers");

        runtime.deregister(&GuestId::from("extra"))?;
        runtime.register("extra", artifact("guest_link_extra_wasm.wasm")?).await?;
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
            .register("router", artifact("guest_link_extra_wasm.wasm")?)
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
            .register("extra", artifact("conformance_wasm.wasm")?)
            .await
            .expect_err("a guest with unsatisfied imports must fail registration");
        assert!(
            runtime.registry().get(&GuestId::from("extra")).is_none(),
            "a failed registration must not publish the guest"
        );

        // The id is fully reusable: a valid registration under it succeeds.
        runtime.register("extra", artifact("guest_link_extra_wasm.wasm")?).await?;
        let echoed = call_router_to(&runtime, "extra", "hello").await?;
        assert_eq!(echoed, "extra echoes from extra: hello");

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
        runtime.register("extra", artifact("guest_link_extra_wasm.wasm")?).await?;
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
