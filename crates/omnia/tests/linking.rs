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
use omnia::wasmtime::{StoreLimits, StoreLimitsBuilder};
use omnia::wasmtime_wasi::{ResourceTable, WasiCtx, WasiCtxBuilder, WasiCtxView, WasiView};
use omnia::{
    GuestId, HasLimits, LinkClient, Registry, Runtime, RuntimeOptions, WrpcCtxView, WrpcState,
    WrpcView, create_from_manifest, serve_links,
};

/// Per-store context mirroring the macro-generated `StoreCtx`: a WASI view plus
/// the wRPC view that host-mediated dispatch encodes and serves through.
struct TestCtx {
    table: ResourceTable,
    wasi: WasiCtx,
    limits: StoreLimits,
    wrpc: WrpcState,
}

impl WasiView for TestCtx {
    fn ctx(&mut self) -> WasiCtxView<'_> {
        WasiCtxView {
            ctx: &mut self.wasi,
            table: &mut self.table,
        }
    }
}

impl HasLimits for TestCtx {
    fn limits(&mut self) -> &mut StoreLimits {
        &mut self.limits
    }
}

impl WrpcView for TestCtx {
    type Invoke = LinkClient;

    fn wrpc(&mut self) -> WrpcCtxView<'_, LinkClient> {
        self.wrpc.view(&mut self.table)
    }
}

/// A minimal [`Runtime`] over the linking registry that counts guest store
/// creations, so the test can assert instance-per-call.
#[derive(Clone)]
struct TestRuntime {
    registry: Arc<Registry<TestCtx>>,
    stores_built: Arc<AtomicUsize>,
}

impl Runtime for TestRuntime {
    type StoreCtx = TestCtx;

    fn store(&self) -> TestCtx {
        // Each fresh guest instance (caller or dispatched callee) draws one store
        // from here, so this counter is the instance-per-call witness.
        self.stores_built.fetch_add(1, Ordering::SeqCst);
        TestCtx {
            table: ResourceTable::new(),
            wasi: WasiCtxBuilder::new().build(),
            limits: StoreLimitsBuilder::new()
                .memory_size(self.registry.options().max_memory_bytes)
                .build(),
            wrpc: WrpcState::new(),
        }
    }

    fn registry(&self) -> &Registry<Self::StoreCtx> {
        &self.registry
    }

    fn options(&self) -> &RuntimeOptions {
        self.registry.options()
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
async fn call_run(runtime: &TestRuntime, message: &str) -> Result<String> {
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

    // A manifest mirroring examples/linking/omni.toml, with absolute source paths
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

    let compiled =
        create_from_manifest::<TestCtx>(&manifest_path, &[]).await.context("building runtime")?;
    let registry = compiled.build_registry().context("assembling registry")?;
    let runtime = TestRuntime {
        registry: Arc::new(registry),
        stores_built: Arc::new(AtomicUsize::new(0)),
    };

    // Wire the serve side of `omnia:link/echo` (responder) and bind the
    // in-process carrier — the work the generated `start()` does for a real
    // deployment.
    serve_links(&runtime).await.context("wiring link serve side")?;

    // Two calls prove the multi-use carrier (a fresh frame connection per call)
    // and instance-per-call: each dispatch instantiates the responder exactly
    // once on a new store.
    for message in ["hello", "world"] {
        let before = runtime.stores_built.load(Ordering::SeqCst);
        let echoed = call_run(&runtime, message).await?;
        let built = runtime.stores_built.load(Ordering::SeqCst) - before;

        assert_eq!(echoed, format!("responder echoes: {message}"));
        // One store for the router (caller) and exactly one for the responder
        // (callee): the callee is instantiated fresh for this single call.
        assert_eq!(built, 2, "each call builds one caller and one callee store");
    }

    let _ = std::fs::remove_file(&manifest_path);
    Ok(())
}
