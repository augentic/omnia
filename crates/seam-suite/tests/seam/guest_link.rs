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
use omnia::{DeploymentBuilder, GuestId, MountRegistry, Runtime, serve_links};
use omnia_testkit::{find_guest, temp_manifest};

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

/// Build the two-guest deployment and wire the serve side, returning the
/// runtime plus the shared bundle-clone counter.
async fn build_runtime() -> Result<(Runtime<Counter>, Arc<AtomicUsize>)> {
    let responder = find_guest("guest_link_responder_wasm.wasm");
    let router = find_guest("guest_link_router_wasm.wasm");

    // A manifest mirroring examples/guest-link/omnia.toml, with absolute source paths
    // so it resolves regardless of the working directory.
    let manifest = temp_manifest(&format!(
        "[[guest]]\n\
         id = \"responder\"\n\
         source.path = \"{responder}\"\n\n\
         [[guest]]\n\
         id = \"router\"\n\
         source.path = \"{router}\"\n\
         link = [\"omnia:link/echo\"]\n",
        responder = responder.display(),
        router = router.display(),
    ))?;

    let deployment = DeploymentBuilder::new()
        .config(manifest.path().to_path_buf())
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
