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

use anyhow::{Context as _, Result, bail};
use futures::FutureExt as _;
use omnia::wasmtime::component::Val;
use omnia::wasmtime::{StoreLimits, StoreLimitsBuilder};
use omnia::wasmtime_wasi::{ResourceTable, WasiCtx, WasiCtxBuilder, WasiCtxView, WasiView};
use omnia::{
    Backend, Compiled, GuestId, HasLimits, LinkClient, Registry, Runtime, RuntimeOptions,
    WrpcCtxView, WrpcState, WrpcView, create_from_manifest,
};
use omnia_wasi_model::{
    BackendAnswer, ConnectOptions, FutureResult, ModelDefault, Prompt, Recording, ToolHost,
    WasiModel, WasiModelCtx, WasiModelCtxView, WasiModelView,
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
    model: Box<dyn WasiModelCtx>,
}

impl WasiView for TestCtx {
    fn ctx(&mut self) -> WasiCtxView<'_> {
        WasiCtxView { ctx: &mut self.wasi, table: &mut self.table }
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
        WasiModelCtxView { ctx: self.model.as_mut(), table: &mut self.table }
    }
}

/// A minimal [`Runtime`] over the model registry; `store()` installs the backend
/// the current phase configured.
#[derive(Clone)]
struct TestRuntime {
    registry: Arc<Registry<TestCtx>>,
    backend: BackendFactory,
}

impl Runtime for TestRuntime {
    type StoreCtx = TestCtx;

    fn store(&self) -> TestCtx {
        TestCtx {
            table: ResourceTable::new(),
            wasi: WasiCtxBuilder::new().build(),
            limits: StoreLimitsBuilder::new()
                .memory_size(self.registry.options().max_memory_bytes)
                .build(),
            wrpc: WrpcState::new(),
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
    fn complete(&self, _prompt: Prompt, _tool_host: Arc<dyn ToolHost>) -> FutureResult<BackendAnswer> {
        let value = self.value.clone();
        async move { Ok(BackendAnswer { value, transcript: None }) }.boxed()
    }
}

/// The `target/` directory: the test executable lives at
/// `<target>/<profile>/deps/<exe>`.
fn target_dir() -> PathBuf {
    let exe = std::env::current_exe().expect("test executable has a path");
    exe.ancestors().nth(3).expect("test exe sits at <target>/<profile>/deps/<exe>").to_path_buf()
}

/// Locate the built guest component, preferring the debug profile.
fn guest_wasm(target: &Path) -> Option<PathBuf> {
    ["debug", "release"]
        .into_iter()
        .map(|profile| {
            target.join("wasm32-wasip2").join(profile).join("examples").join("model_wasm.wasm")
        })
        .find(|path| path.exists())
}

/// Build the model runtime for `wasm`, linking `WasiModel`, and return the shared
/// registry.
async fn build_registry(wasm: &Path) -> Result<Arc<Registry<TestCtx>>> {
    // A one-guest manifest with an absolute source path.
    let manifest_path =
        std::env::temp_dir().join(format!("omnia-model-{}.toml", std::process::id()));
    let manifest = format!("[[guest]]\nid = \"model\"\nsource.path = \"{}\"\n", wasm.display());
    std::fs::write(&manifest_path, manifest).context("writing test manifest")?;

    let mut compiled: Compiled<TestCtx> =
        create_from_manifest(&manifest_path, &[]).await.context("building runtime")?;
    compiled.link(WasiModel).context("linking WasiModel")?;
    let registry = compiled.build_registry().context("assembling registry")?;

    let _ = std::fs::remove_file(&manifest_path);
    Ok(Arc::new(registry))
}

/// Instantiate the guest fresh and drive its async `run` export.
async fn call_run(runtime: &TestRuntime) -> Result<String> {
    let guest =
        runtime.registry().get(&GuestId::from("model")).context("model guest is registered")?;
    let mut store = runtime.build_store(runtime.store());
    let instance =
        runtime.instantiate(guest.instance_pre(), &mut store).await.context("instantiating guest")?;
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
    let Some(wasm) = guest_wasm(&target_dir()) else {
        eprintln!(
            "skipping `replays_completion_with_no_network`: model guest not built. Run:\n  \
             cargo build -p examples --example model-wasm --target wasm32-wasip2"
        );
        return Ok(());
    };

    let registry = build_registry(&wasm).await?;

    // The answer the recorded run produces and the replay must reproduce.
    let expected = expected_answer();

    // Phase 1 — record: a stub backend answers through `complete`, and the
    // `Recording` wrapper persists the fixture keyed by the guest's real prompt.
    let record_dir =
        std::env::temp_dir().join(format!("omnia-model-fixtures-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&record_dir);
    {
        let value = expected.clone();
        let dir = record_dir.clone();
        let runtime = TestRuntime {
            registry: Arc::clone(&registry),
            backend: Arc::new(move || {
                Box::new(Recording::new(StubBackend { value: value.clone() }, dir.clone()))
            }),
        };
        let answer = call_run(&runtime).await.context("record phase")?;
        assert_eq!(
            serde_json::from_str::<Value>(&answer).context("answer is JSON")?,
            expected,
            "recorded run should return the validated answer"
        );
    }
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
    let wasm = guest_wasm(&target_dir())
        .context("model guest not built; build it before regenerating the fixture")?;
    let registry = build_registry(&wasm).await?;

    let dir = committed_fixtures();
    let _ = std::fs::remove_dir_all(&dir);
    let value = expected_answer();
    let backend_dir = dir.clone();
    let runtime = TestRuntime {
        registry,
        backend: Arc::new(move || {
            Box::new(Recording::new(StubBackend { value: value.clone() }, backend_dir.clone()))
        }),
    };
    let answer = call_run(&runtime).await.context("recording example fixture")?;
    eprintln!("recorded example fixture into {}: {answer}", dir.display());
    Ok(())
}

/// Replay the guest with a `ModelDefault` backend loaded from `dir`.
async fn replay_from(registry: &Arc<Registry<TestCtx>>, dir: &Path) -> Result<String> {
    let backend = ModelDefault::connect_with(ConnectOptions { replay_dir: dir.to_path_buf() })
        .await
        .context("connecting replay backend")?;
    let runtime = TestRuntime {
        registry: Arc::clone(registry),
        backend: Arc::new(move || Box::new(backend.clone())),
    };
    call_run(&runtime).await
}
