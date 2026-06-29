//! Integration test for host-mediated dynamic linking (Phase 2 of
//! `rfcs/guest-registry.md`).
//!
//! Builds the `examples/linking` deployment — `router` imports `omnia:link/echo`,
//! `responder` exports it — wires the serve side, and drives `router.run`. It
//! proves the end-to-end dispatch: the call routes through the floor's selector
//! to the responder over the in-process wRPC carrier, the responder is
//! instantiated fresh per call (instance-per-call), and the typed result returns
//! to the caller. Two calls confirm the multi-use carrier (a fresh frame
//! connection per call).
//!
//! The guest components must be built first; the test skips (rather than fails)
//! when they are absent, because `cargo make ci` cleans the target directory
//! before running tests:
//!
//! ```bash
//! cargo build -p examples --example linking-responder-wasm \
//!   --example linking-router-wasm --target wasm32-wasip2
//! ```

#![cfg(not(target_arch = "wasm32"))]

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use anyhow::{Context as _, Result, bail};
use omnia::wasmtime::component::Val;
use omnia::{DeploymentBuilder, GuestId, Runtime, WorkingTreeRegistry, serve_links};

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

/// The `target/` directory: the test executable lives at
/// `<target>/<profile>/deps/<exe>`.
fn target_dir() -> PathBuf {
    let exe = std::env::current_exe().expect("test executable has a path");
    exe.ancestors().nth(3).expect("test exe sits at <target>/<profile>/deps/<exe>").to_path_buf()
}

/// Locate a built guest component by file name, preferring the debug profile.
fn guest_wasm(target: &Path, file: &str) -> Option<PathBuf> {
    ["debug", "release"]
        .into_iter()
        .map(|profile| target.join("wasm32-wasip2").join(profile).join("examples").join(file))
        .find(|path| path.exists())
}

/// Instantiate the router fresh, call its `run` export with `message`, and return
/// the echoed string.
async fn call_run(runtime: &Runtime<Counter>, message: &str) -> Result<String> {
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

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn router_dispatches_to_responder() -> Result<()> {
    let target = target_dir();
    let (Some(responder), Some(router)) = (
        guest_wasm(&target, "linking_responder_wasm.wasm"),
        guest_wasm(&target, "linking_router_wasm.wasm"),
    ) else {
        eprintln!(
            "skipping `router_dispatches_to_responder`: linking guests not built. Run:\n  \
             cargo build -p examples --example linking-responder-wasm \
             --example linking-router-wasm --target wasm32-wasip2"
        );
        return Ok(());
    };

    // A manifest mirroring examples/linking/omnia.toml, with absolute source paths
    // so it resolves regardless of the working directory.
    let manifest_path =
        std::env::temp_dir().join(format!("omnia-linking-{}.toml", std::process::id()));
    let manifest = format!(
        "[[guest]]\n\
         id = \"responder\"\n\
         source.path = \"{responder}\"\n\n\
         [[guest]]\n\
         id = \"router\"\n\
         source.path = \"{router}\"\n\
         link = [\"omnia:link/echo\"]\n",
        responder = responder.display(),
        router = router.display(),
    );
    std::fs::write(&manifest_path, manifest).context("writing test manifest")?;

    let deployment = DeploymentBuilder::new()
        .config(manifest_path.clone())
        .build::<TestCtx>()
        .await
        .context("building runtime")?;
    let registry = deployment.build().context("assembling registry")?;
    let clones = Arc::new(AtomicUsize::new(0));
    let runtime = Runtime::<Counter>::from_parts(
        Arc::new(registry),
        Vec::new(),
        Arc::new(WorkingTreeRegistry::default()),
        Counter {
            clones: Arc::clone(&clones),
        },
    );

    // Wire the serve side of `omnia:link/echo` (responder) and bind the
    // in-process carrier — the work the generated `start()` does for a real
    // deployment.
    serve_links(&runtime).await.context("wiring link serve side")?;

    // Two calls prove the multi-use carrier (a fresh frame connection per call)
    // and instance-per-call: each dispatch instantiates the responder fresh on a
    // new store. The bundle clone count rises by a fixed, nonzero amount per call
    // (router caller store + responder callee store); equal deltas across the two
    // calls witness that the second call rebuilds the callee rather than reusing
    // a cached one.
    let mut per_call: Option<usize> = None;
    for message in ["hello", "world"] {
        let before = clones.load(Ordering::SeqCst);
        let echoed = call_run(&runtime, message).await?;
        let delta = clones.load(Ordering::SeqCst) - before;

        assert_eq!(echoed, format!("responder echoes: {message}"));
        assert!(delta > 0, "each call builds at least one store");
        match per_call {
            None => per_call = Some(delta),
            Some(expected) => {
                assert_eq!(delta, expected, "each call does identical work (instance-per-call)");
            }
        }
    }

    let _ = std::fs::remove_file(&manifest_path);
    Ok(())
}
