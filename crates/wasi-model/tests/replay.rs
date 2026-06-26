//! Integration test for `wasi-model` Phase 1 — the run-1 (replay) acceptance
//! gate (`rfcs/wasi-model.md` §6).
//!
//! Builds the `examples/model` `complete` guest, links the `WasiModel` host, and
//! drives the guest's `run` export across the real WIT boundary. It proves the
//! Layer 1 invariant end-to-end:
//!
//! 1. **record** — a stub backend (no network) answers once through `complete`;
//!    the `Recording` wrapper writes a fixture keyed by the guest's real prompt;
//! 2. **replay** — `ModelDefault` loaded from that directory serves the recorded,
//!    validated answer for the same guest with no backend at all;
//! 3. **checked-in fixture** — `ModelDefault` loaded from `examples/model/fixtures`
//!    replays the guest, proving the committed fixture still matches the guest.
//!
//! The guest component must be built first; the test skips (rather than fails)
//! when it is absent, because `cargo make ci` cleans the target directory before
//! running tests:
//!
//! ```bash
//! cargo build -p examples --example model-wasm --target wasm32-wasip2
//! ```

#![cfg(not(target_arch = "wasm32"))]

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use anyhow::{Context as _, Result, bail};
use futures::FutureExt as _;
use omnia::wasmtime::component::Val;
use omnia::wasmtime::{StoreLimits, StoreLimitsBuilder};
use omnia::wasmtime_wasi::{ResourceTable, WasiCtx, WasiCtxBuilder, WasiCtxView, WasiView};
use omnia::{
    Backend, Compiled, GuestId, HasLimits, HostDispatch, LinkClient, Registry, RegistryBuilder,
    Runtime, RuntimeOptions, WrpcCtxView, WrpcState, WrpcView,
};
use omnia_wasi_model::{
    BackendAnswer, ConnectOptions, FutureResult, ModelDefault, Prompt, Recording, Reference,
    ToolHost, WasiModel, WasiModelCtx, WasiModelCtxView, WasiModelView,
};
use serde_json::{Value, json};

/// A factory the test runtime calls per store to install a fresh backend.
type BackendFactory = Arc<dyn Fn() -> Box<dyn WasiModelCtx> + Send + Sync>;

/// Per-store context: the WASI + wRPC views the floor needs, plus the swappable
/// model backend the test installs (record vs replay).
struct TestCtx {
    table: ResourceTable,
    wasi: WasiCtx,
    limits: StoreLimits,
    wrpc: WrpcState,
    host_dispatch: Arc<dyn HostDispatch>,
    model: Box<dyn WasiModelCtx>,
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

impl WasiModelView for TestCtx {
    fn model(&mut self) -> WasiModelCtxView<'_> {
        WasiModelCtxView {
            ctx: self.model.as_mut(),
            table: &mut self.table,
            host_dispatch: Arc::clone(&self.host_dispatch),
        }
    }
}

/// A minimal [`Runtime`] over the model registry; `store()` installs the backend
/// the current phase configured and counts store creations so a test can assert
/// instance-per-call through `resolve`.
#[derive(Clone)]
struct TestRuntime {
    registry: Arc<Registry<TestCtx>>,
    backend: BackendFactory,
    stores_built: Arc<AtomicUsize>,
}

impl Runtime for TestRuntime {
    type StoreCtx = TestCtx;

    fn store(&self) -> TestCtx {
        // Each fresh guest instance (the `complete` caller or a dispatched
        // `resolve` callee) draws one store here — the instance-per-call witness.
        self.stores_built.fetch_add(1, Ordering::SeqCst);
        TestCtx {
            table: ResourceTable::new(),
            wasi: WasiCtxBuilder::new().build(),
            limits: StoreLimitsBuilder::new()
                .memory_size(self.registry.options().max_memory_bytes)
                .build(),
            wrpc: WrpcState::new(),
            host_dispatch: Arc::new(self.clone()),
            model: (self.backend)(),
        }
    }

    fn registry(&self) -> &Registry<Self::StoreCtx> {
        &self.registry
    }

    fn options(&self) -> &RuntimeOptions {
        self.registry.options()
    }
}

/// A backend that always answers `value`, with no network (the record source).
#[derive(Debug, Clone)]
struct StubBackend {
    value: Value,
}

impl WasiModelCtx for StubBackend {
    fn complete(
        &self, _prompt: Prompt, _tool_host: Arc<dyn ToolHost>,
    ) -> FutureResult<BackendAnswer> {
        let value = self.value.clone();
        async move {
            Ok(BackendAnswer {
                value,
                transcript: None,
            })
        }
        .boxed()
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

/// Build the model runtime for `wasm`, linking `WasiModel`, and return the shared
/// registry.
fn registry(wasm: &Path) -> Result<Arc<Registry<TestCtx>>> {
    // A one-guest manifest with an absolute source path.
    let manifest_path =
        std::env::temp_dir().join(format!("omnia-model-{}.toml", std::process::id()));
    let manifest = format!("[[guest]]\nid = \"model\"\nsource.path = \"{}\"\n", wasm.display());
    std::fs::write(&manifest_path, manifest).context("writing test manifest")?;

    let mut compiled: Compiled<TestCtx> = RegistryBuilder::new()
        .config(manifest_path.clone())
        .compile()
        .context("building runtime")?;
    compiled.link::<WasiModel>().context("linking WasiModel")?;
    let registry = compiled.build().context("assembling registry")?;

    let _ = std::fs::remove_file(&manifest_path);
    Ok(Arc::new(registry))
}

/// Instantiate the guest fresh and drive its async `run` export.
async fn call_run(runtime: &TestRuntime) -> Result<String> {
    let guest =
        runtime.registry().get(&GuestId::from("model")).context("model guest is registered")?;
    let mut store = runtime.build_store(runtime.store());
    let instance = runtime
        .instantiate(guest.instance_pre(), &mut store)
        .await
        .context("instantiating guest")?;
    let run = instance.get_func(&mut store, "run").context("guest exports `run`")?;

    let mut results = vec![Val::String(String::new())];
    run.call_async(&mut store, &[], &mut results)
        .await
        .map_err(anyhow::Error::from)
        .context("calling model.run")?;

    match results.into_iter().next() {
        Some(Val::String(answer)) => Ok(answer),
        other => bail!("model.run returned a non-string result: {other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn replays_completion_with_no_network() -> Result<()> {
    let Some(wasm) = guest_wasm(&target_dir(), "model_wasm.wasm") else {
        eprintln!(
            "skipping `replays_completion_with_no_network`: model guest not built. Run:\n  \
             cargo build -p examples --example model-wasm --target wasm32-wasip2"
        );
        return Ok(());
    };

    let registry = registry(&wasm)?;

    // The answer the recorded run produces and the replay must reproduce.
    let expected = expected_answer();

    // Phase 1 — record: a stub backend answers through `complete`, and the
    // `Recording` wrapper persists the fixture keyed by the guest's real prompt.
    let record_dir =
        std::env::temp_dir().join(format!("omnia-model-fixtures-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&record_dir);
    record_phase(&registry, &record_dir, &expected).await?;
    let written = std::fs::read_dir(&record_dir)
        .context("reading record dir")?
        .filter_map(std::result::Result::ok)
        .count();
    assert_eq!(written, 1, "recording should write exactly one fixture");

    // Phase 2 — replay: `ModelDefault` loaded from the recorded directory serves
    // the same guest with no backend and no network.
    let replayed = replay_from(&registry, &record_dir).await.context("replay phase")?;
    assert_eq!(
        serde_json::from_str::<Value>(&replayed).context("answer is JSON")?,
        expected,
        "replayed run should reproduce the recorded answer"
    );
    let _ = std::fs::remove_dir_all(&record_dir);

    // Phase 3 — checked-in fixture: the committed example fixture still matches
    // the guest's prompt (guards against keying drift over time).
    let from_committed =
        replay_from(&registry, &committed_fixtures()).await.context("committed fixture")?;
    assert_eq!(
        serde_json::from_str::<Value>(&from_committed).context("answer is JSON")?,
        expected,
        "checked-in example fixture should replay the guest"
    );

    Ok(())
}

/// The answer the example guest's prompt resolves to — the value the stub
/// backend records and every replay must reproduce. Shared so the regeneration
/// helper and the acceptance test cannot drift.
fn expected_answer() -> Value {
    json!({ "verdict": "pass", "reason": "the bounds check is correct" })
}

/// The checked-in example fixture directory (`examples/model/fixtures`).
fn committed_fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/model/fixtures")
}

/// Regenerate the checked-in example fixture from the live guest.
///
/// Ignored by default because it writes into the source tree; run it after the
/// example guest's prompt changes so `examples/model/fixtures` stays in step:
///
/// ```bash
/// cargo build -p examples --example model-wasm --target wasm32-wasip2
/// cargo test -p omnia-wasi-model --test replay -- --ignored record_example_fixture
/// ```
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "writes into the source tree; run manually to regenerate the fixture"]
async fn record_example_fixture() -> Result<()> {
    let wasm = guest_wasm(&target_dir(), "model_wasm.wasm")
        .context("model guest not built; build it before regenerating the fixture")?;
    let registry = registry(&wasm)?;

    let dir = committed_fixtures();
    let _ = std::fs::remove_dir_all(&dir);
    let value = expected_answer();
    let backend_dir = dir.clone();
    let runtime = TestRuntime {
        registry,
        backend: Arc::new(move || {
            Box::new(Recording::new(StubBackend { value: value.clone() }, backend_dir.clone()))
        }),
        stores_built: Arc::new(AtomicUsize::new(0)),
    };
    let answer = call_run(&runtime).await.context("recording example fixture")?;
    eprintln!("recorded example fixture into {}: {answer}", dir.display());
    Ok(())
}

/// Record the guest once: a stub backend answers through `complete`, and the
/// `Recording` wrapper persists the fixture keyed by the guest's real prompt.
async fn record_phase(
    registry: &Arc<Registry<TestCtx>>, dir: &Path, expected: &Value,
) -> Result<()> {
    let value = expected.clone();
    let backend_dir = dir.to_path_buf();
    let runtime = TestRuntime {
        registry: Arc::clone(registry),
        backend: Arc::new(move || {
            Box::new(Recording::new(StubBackend { value: value.clone() }, backend_dir.clone()))
        }),
        stores_built: Arc::new(AtomicUsize::new(0)),
    };
    let answer = call_run(&runtime).await.context("record phase")?;
    assert_eq!(
        serde_json::from_str::<Value>(&answer).context("answer is JSON")?,
        *expected,
        "recorded run should return the validated answer"
    );
    Ok(())
}

/// Replay the guest with a `ModelDefault` backend loaded from `dir`.
async fn replay_from(registry: &Arc<Registry<TestCtx>>, dir: &Path) -> Result<String> {
    let backend = ModelDefault::connect_with(ConnectOptions {
        replay_dir: dir.to_path_buf(),
    })
    .await
    .context("connecting replay backend")?;
    let runtime = TestRuntime {
        registry: Arc::clone(registry),
        backend: Arc::new(move || Box::new(backend.clone())),
        stores_built: Arc::new(AtomicUsize::new(0)),
    };
    call_run(&runtime).await
}

/// A backend that drives the host→guest `resolve` path with no network: it calls
/// `tool_host.resolve` for two references and folds the returned bytes into a
/// JSON-object answer. The prompt's `grants.references` (the guest sets it to
/// `"shelf"`) routes each call to a fresh `shelf` instance.
#[derive(Debug)]
struct ResolvingStub;

impl WasiModelCtx for ResolvingStub {
    fn complete(
        &self, _prompt: Prompt, tool_host: Arc<dyn ToolHost>,
    ) -> FutureResult<BackendAnswer> {
        async move {
            let alpha = tool_host
                .resolve(Reference {
                    name: "alpha".to_owned(),
                })
                .await?;
            let beta = tool_host
                .resolve(Reference {
                    name: "beta".to_owned(),
                })
                .await?;
            Ok(BackendAnswer {
                value: json!({
                    "alpha": String::from_utf8(alpha).context("alpha bytes are utf-8")?,
                    "beta": String::from_utf8(beta).context("beta bytes are utf-8")?,
                }),
                transcript: None,
            })
        }
        .boxed()
    }
}

/// Build a two-guest registry (`model` + `shelf`) for the resolve path, linking
/// `WasiModel`. The `shelf` needs no `link` declaration: host→guest `resolve` is
/// a direct instantiate-and-call.
fn build_registry(model: &Path, shelf: &Path) -> Result<Arc<Registry<TestCtx>>> {
    let manifest_path =
        std::env::temp_dir().join(format!("omnia-model-resolve-{}.toml", std::process::id()));
    let manifest = format!(
        "[[guest]]\nid = \"model\"\nsource.path = \"{model}\"\n\n\
         [[guest]]\nid = \"shelf\"\nsource.path = \"{shelf}\"\n",
        model = model.display(),
        shelf = shelf.display(),
    );
    std::fs::write(&manifest_path, manifest).context("writing resolve test manifest")?;

    let mut compiled: Compiled<TestCtx> = RegistryBuilder::new()
        .config(manifest_path.clone())
        .compile()
        .context("building runtime")?;
    compiled.link::<WasiModel>().context("linking WasiModel")?;
    let registry = compiled.build().context("assembling registry")?;

    let _ = std::fs::remove_file(&manifest_path);
    Ok(Arc::new(registry))
}

/// Phase 2a — the CI-runnable `resolve` acceptance gate (no network).
///
/// A stub backend drives the host→guest `resolve` path for the guest's
/// `grants.references = "shelf"` prompt. It proves Task A (the `dispatch_to_guest`
/// entry point) + Task B (the `BoundToolHost` wiring) deterministically: every
/// `resolve` lands a **fresh `shelf` instance** (instance-per-call witness) and
/// the bytes round-trip through the seam.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn resolve_dispatches_to_a_fresh_shelf_per_call() -> Result<()> {
    let target = target_dir();
    let (Some(model), Some(shelf)) =
        (guest_wasm(&target, "model_wasm.wasm"), guest_wasm(&target, "model_shelf_wasm.wasm"))
    else {
        eprintln!(
            "skipping `resolve_dispatches_to_a_fresh_shelf_per_call`: model/shelf guests not \
             built. Run:\n  cargo build -p examples --example model-wasm \
             --example model-shelf-wasm --target wasm32-wasip2"
        );
        return Ok(());
    };

    let registry = build_registry(&model, &shelf)?;
    let runtime = TestRuntime {
        registry,
        backend: Arc::new(|| Box::new(ResolvingStub)),
        stores_built: Arc::new(AtomicUsize::new(0)),
    };

    let before = runtime.stores_built.load(Ordering::SeqCst);
    let answer = call_run(&runtime).await.context("driving the resolve guest")?;
    let built = runtime.stores_built.load(Ordering::SeqCst) - before;

    // Byte round-trip: each reference reached the `shelf` and came back
    // transformed, so the typed bytes crossed the host→guest seam in both
    // directions.
    assert_eq!(
        serde_json::from_str::<Value>(&answer).context("answer is JSON")?,
        json!({ "alpha": "shelf:alpha", "beta": "shelf:beta" }),
        "resolved bytes should round-trip through the host→guest seam"
    );

    // Instance-per-call: one store for the `model` guest (the `complete` caller)
    // plus one fresh `shelf` store per `resolve`. The shelf is never reused
    // across calls and can never re-enter the caller.
    assert_eq!(built, 3, "one caller store + one fresh shelf store per resolve (two resolves)");

    Ok(())
}
