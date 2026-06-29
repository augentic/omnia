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
use omnia::{
    Backend, Deployment, DeploymentBuilder, GuestId, MountRegistry, Registry, ResolvedPreopen,
    Runtime,
};
use omnia_wasi_model::{
    BackendAnswer, ConnectOptions, FutureResult, HasModel, ModelDefault, Prompt, Recording,
    Reference, ToolHost, WasiModel, WasiModelCtx,
};
use serde_json::{Value, json};

/// A factory the test bundle calls per clone to mint a fresh backend.
type BackendFactory = Arc<dyn Fn() -> Box<dyn WasiModelCtx> + Send + Sync>;

/// The deployment's backend bundle for the test: the swappable model backend the
/// test installs (record vs replay). Its [`HasModel`] impl is what
/// `omnia::StoreCtx<TestBundle>` reads to serve `wasi-model`.
///
/// The library [`Runtime::store`] clones the bundle to build each per-guest
/// store, so the bundle's [`Clone`] mints a fresh backend (replacing the old
/// per-store factory call) and bumps `stores_built` — turning bundle cloning
/// into the instance-creation witness the `resolve` path asserts on.
struct TestBundle {
    backend: BackendFactory,
    model: Box<dyn WasiModelCtx>,
    stores_built: Arc<AtomicUsize>,
}

impl Clone for TestBundle {
    fn clone(&self) -> Self {
        self.stores_built.fetch_add(1, Ordering::SeqCst);
        Self {
            backend: Arc::clone(&self.backend),
            model: (self.backend)(),
            stores_built: Arc::clone(&self.stores_built),
        }
    }
}

impl HasModel for TestBundle {
    fn model_ctx(&mut self) -> &mut dyn WasiModelCtx {
        &mut *self.model
    }
}

/// Per-store context: the library [`omnia::StoreCtx`] over [`TestBundle`]. The
/// fixed `WasiView` / `WrpcView` / `HasLimits` views come from `omnia`, and the
/// `WasiModel` host view is the blanket impl over `StoreCtx<B>` that reads
/// `TestBundle`'s [`HasModel`].
type TestCtx = omnia::StoreCtx<TestBundle>;

/// Assemble a model [`Runtime`] over `registry` from already-built parts: a
/// backend `factory` (cloned fresh into every store) and the `working_trees`
/// mounts preopened into each store (RFC-55; empty for the `resolve` path, a
/// single `.` mount for the completion path so the guest lends a tree). The
/// shared `stores_built` counts bundle clones — the instance-creation witness.
fn model_runtime(
    registry: Arc<Registry<TestCtx>>, backend: BackendFactory, stores_built: Arc<AtomicUsize>,
    working_trees: Arc<MountRegistry>,
) -> Runtime<TestBundle> {
    let bundle = TestBundle {
        model: backend(),
        backend,
        stores_built,
    };
    Runtime::from_parts(registry, Vec::new(), working_trees, bundle)
}

/// A single read-only working-tree mount named `.` over a fresh temp directory —
/// the shape `omnia.toml`'s `[[mount]]` resolves to. The example guest reads it
/// via `preopens.get-directories()` and lends it through `grants.working-tree`,
/// so the recorded prompt carries `working_tree_lent = true`. The directory's
/// identity is irrelevant to the replay key (only the boolean marker lands
/// there), so any real directory serves.
fn working_tree_mount() -> (PathBuf, Arc<MountRegistry>) {
    let dir = std::env::temp_dir().join(format!("omnia-model-tree-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("creating the working-tree mount dir");
    let registry =
        MountRegistry::open(vec![ResolvedPreopen::new(".".to_owned(), dir.clone(), false)])
            .expect("opening the working-tree mount");
    (dir, Arc::new(registry))
}

/// An empty registry — no preopens, the default for paths that don't exercise a
/// working tree.
fn no_working_trees() -> Arc<MountRegistry> {
    Arc::new(MountRegistry::default())
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
async fn registry(wasm: &Path) -> Result<Arc<Registry<TestCtx>>> {
    // A one-guest manifest with an absolute source path.
    let manifest_path =
        std::env::temp_dir().join(format!("omnia-model-{}.toml", std::process::id()));
    let manifest = format!("[[guest]]\nid = \"model\"\nsource.path = \"{}\"\n", wasm.display());
    std::fs::write(&manifest_path, manifest).context("writing test manifest")?;

    let mut deployment: Deployment<TestCtx> = DeploymentBuilder::new()
        .config(manifest_path.clone())
        .build()
        .await
        .context("building runtime")?;
    deployment.host::<WasiModel, TestBundle>().context("linking WasiModel")?;
    let registry = deployment.build().context("assembling registry")?;

    let _ = std::fs::remove_file(&manifest_path);
    Ok(Arc::new(registry))
}

/// Instantiate the guest fresh and drive its async `run` export.
async fn call_run(runtime: &Runtime<TestBundle>) -> Result<String> {
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

    let registry = registry(&wasm).await?;

    // The answer the recorded run produces and the replay must reproduce.
    let expected = expected_answer();

    // The completion path preopens a working tree the example guest lends
    // (RFC-55), so the recorded prompt carries `working_tree_lent = true` and the
    // floor resolves the lent descriptor back to this mount by identity.
    let (mount_dir, working_trees) = working_tree_mount();

    // Phase 1 — record: a stub backend answers through `complete`, and the
    // `Recording` wrapper persists the fixture keyed by the guest's real prompt.
    let record_dir =
        std::env::temp_dir().join(format!("omnia-model-fixtures-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&record_dir);
    record_phase(&registry, &record_dir, &expected, &working_trees).await?;
    let fixtures: Vec<PathBuf> = std::fs::read_dir(&record_dir)
        .context("reading record dir")?
        .filter_map(std::result::Result::ok)
        .map(|entry| entry.path())
        .collect();
    assert_eq!(fixtures.len(), 1, "recording should write exactly one fixture");

    // The replay key reduces the lent tree to a single `working_tree_lent`
    // boolean — never the descriptor's identity or the mount's host path.
    let recorded = std::fs::read_to_string(&fixtures[0]).context("reading recorded fixture")?;
    let fixture: Value = serde_json::from_str(&recorded).context("fixture is JSON")?;
    assert_eq!(
        fixture["key_prompt"]["grants"]["working_tree_lent"],
        json!(true),
        "a lent tree keys as `working_tree_lent: true`"
    );
    assert!(
        !recorded.contains(mount_dir.to_string_lossy().as_ref()),
        "the fixture key must not leak the mount's host path"
    );

    // Phase 2 — replay: `ModelDefault` loaded from the recorded directory serves
    // the same guest with no backend and no network.
    let replayed =
        replay_from(&registry, &record_dir, &working_trees).await.context("replay phase")?;
    assert_eq!(
        serde_json::from_str::<Value>(&replayed).context("answer is JSON")?,
        expected,
        "replayed run should reproduce the recorded answer"
    );
    let _ = std::fs::remove_dir_all(&record_dir);

    // Phase 3 — checked-in fixture: the committed example fixture still matches
    // the guest's prompt (guards against keying drift over time).
    let from_committed = replay_from(&registry, &committed_fixtures(), &working_trees)
        .await
        .context("committed fixture")?;
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
    let registry = registry(&wasm).await?;

    let dir = committed_fixtures();
    let _ = std::fs::remove_dir_all(&dir);
    let value = expected_answer();
    let backend_dir = dir.clone();
    // Preopen the same `.` mount the example's `omnia.toml` declares, so the
    // regenerated fixture matches what the guest lends at runtime
    // (`working_tree_lent = true`).
    let (_mount_dir, working_trees) = working_tree_mount();
    let runtime = model_runtime(
        registry,
        Arc::new(move || {
            Box::new(Recording::new(StubBackend { value: value.clone() }, backend_dir.clone()))
        }),
        Arc::new(AtomicUsize::new(0)),
        working_trees,
    );
    let answer = call_run(&runtime).await.context("recording example fixture")?;
    eprintln!("recorded example fixture into {}: {answer}", dir.display());
    Ok(())
}

/// Record the guest once: a stub backend answers through `complete`, and the
/// `Recording` wrapper persists the fixture keyed by the guest's real prompt.
async fn record_phase(
    registry: &Arc<Registry<TestCtx>>, dir: &Path, expected: &Value,
    working_trees: &Arc<MountRegistry>,
) -> Result<()> {
    let value = expected.clone();
    let backend_dir = dir.to_path_buf();
    let runtime = model_runtime(
        Arc::clone(registry),
        Arc::new(move || {
            Box::new(Recording::new(StubBackend { value: value.clone() }, backend_dir.clone()))
        }),
        Arc::new(AtomicUsize::new(0)),
        Arc::clone(working_trees),
    );
    let answer = call_run(&runtime).await.context("record phase")?;
    assert_eq!(
        serde_json::from_str::<Value>(&answer).context("answer is JSON")?,
        *expected,
        "recorded run should return the validated answer"
    );
    Ok(())
}

/// Replay the guest with a `ModelDefault` backend loaded from `dir`.
async fn replay_from(
    registry: &Arc<Registry<TestCtx>>, dir: &Path, working_trees: &Arc<MountRegistry>,
) -> Result<String> {
    let backend = ModelDefault::connect_with(ConnectOptions {
        replay_dir: dir.to_path_buf(),
    })
    .await
    .context("connecting replay backend")?;
    let runtime = model_runtime(
        Arc::clone(registry),
        Arc::new(move || Box::new(backend.clone())),
        Arc::new(AtomicUsize::new(0)),
        Arc::clone(working_trees),
    );
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
async fn build_registry(model: &Path, shelf: &Path) -> Result<Arc<Registry<TestCtx>>> {
    let manifest_path =
        std::env::temp_dir().join(format!("omnia-model-resolve-{}.toml", std::process::id()));
    let manifest = format!(
        "[[guest]]\nid = \"model\"\nsource.path = \"{model}\"\n\n\
         [[guest]]\nid = \"shelf\"\nsource.path = \"{shelf}\"\n",
        model = model.display(),
        shelf = shelf.display(),
    );
    std::fs::write(&manifest_path, manifest).context("writing resolve test manifest")?;

    let mut deployment: Deployment<TestCtx> = DeploymentBuilder::new()
        .config(manifest_path.clone())
        .build()
        .await
        .context("building runtime")?;
    deployment.host::<WasiModel, TestBundle>().context("linking WasiModel")?;
    let registry = deployment.build().context("assembling registry")?;

    let _ = std::fs::remove_file(&manifest_path);
    Ok(Arc::new(registry))
}

/// Phase 2a — the CI-runnable `resolve` acceptance gate (no network).
///
/// A stub backend drives the host→guest `resolve` path for the guest's
/// `grants.references = "shelf"` prompt. It proves Task A (the `dispatch`
/// entry point) + Task B (the `BoundToolHost` wiring) deterministically: every
/// `resolve` lands a **fresh `shelf` instance** (instance-per-call witness) and
/// the bytes round-trip through the seam.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn resolve_dispatches() -> Result<()> {
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

    let registry = build_registry(&model, &shelf).await?;
    let stores_built = Arc::new(AtomicUsize::new(0));
    let runtime = model_runtime(
        registry,
        Arc::new(|| Box::new(ResolvingStub)),
        Arc::clone(&stores_built),
        // The `resolve` path doesn't exercise a working tree; the model guest's
        // `get-directories()` sees no mount and lends nothing.
        no_working_trees(),
    );

    // Two completions, each driving two `resolve` dispatches. The library
    // `Runtime::store` clones the bundle to build every per-guest store, so a
    // completion clones the bundle a fixed, nonzero number of times (the `model`
    // caller plus a fresh `shelf` per `resolve`). Equal deltas across the two
    // completions witness instance-per-call: a cached/reused `shelf` would build
    // fewer stores — and clone the bundle fewer times — on the second completion.
    let mut per_call: Option<usize> = None;
    for _ in 0..2 {
        let before = stores_built.load(Ordering::SeqCst);
        let answer = call_run(&runtime).await.context("driving the resolve guest")?;
        let built = stores_built.load(Ordering::SeqCst) - before;

        // Byte round-trip: each reference reached the `shelf` and came back
        // transformed, so the typed bytes crossed the host→guest seam in both
        // directions.
        assert_eq!(
            serde_json::from_str::<Value>(&answer).context("answer is JSON")?,
            json!({ "alpha": "shelf:alpha", "beta": "shelf:beta" }),
            "resolved bytes should round-trip through the host→guest seam"
        );

        assert!(built > 0, "each completion builds at least the caller store");
        match per_call {
            None => per_call = Some(built),
            Some(expected) => {
                assert_eq!(built, expected, "instance-per-call: identical work per completion");
            }
        }
    }

    Ok(())
}

/// A backend that asserts the floor resolved the guest's lent working tree to
/// its mount path — the `local-path` face (RFC-55) the cursor backend consumes.
#[derive(Debug, Clone)]
struct LocalPathProbe {
    expected: PathBuf,
}

impl WasiModelCtx for LocalPathProbe {
    fn complete(
        &self, _prompt: Prompt, tool_host: Arc<dyn ToolHost>,
    ) -> FutureResult<BackendAnswer> {
        let expected = self.expected.clone();
        async move {
            let local = tool_host.local_path().map(Path::to_path_buf);
            anyhow::ensure!(
                local.as_deref() == Some(expected.as_path()),
                "floor must resolve the lent tree to its mount path: got {local:?}, want {}",
                expected.display()
            );
            Ok(BackendAnswer {
                value: json!({ "verdict": "pass", "reason": "local path resolved" }),
                transcript: None,
            })
        }
        .boxed()
    }
}

/// The working-tree `local-path` face end-to-end: the host preopens a `.` mount,
/// the example guest reads it via `preopens.get-directories()` and lends it, and
/// the floor identity-matches it back to the mount — surfacing its host path on
/// the per-completion [`ToolHost`] (RFC-55, what `omnia-cursor` reads).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn working_tree_resolves_to_local_path() -> Result<()> {
    let Some(wasm) = guest_wasm(&target_dir(), "model_wasm.wasm") else {
        eprintln!(
            "skipping `working_tree_resolves_to_local_path`: model guest not built. Run:\n  \
             cargo build -p examples --example model-wasm --target wasm32-wasip2"
        );
        return Ok(());
    };

    let registry = registry(&wasm).await?;
    let (mount_dir, working_trees) = working_tree_mount();
    let expected = mount_dir.clone();
    let runtime = model_runtime(
        registry,
        Arc::new(move || {
            Box::new(LocalPathProbe {
                expected: expected.clone(),
            })
        }),
        Arc::new(AtomicUsize::new(0)),
        working_trees,
    );

    let answer = call_run(&runtime).await.context("driving the local-path probe")?;
    let value: Value = serde_json::from_str(&answer)
        .with_context(|| format!("probe answer should be JSON, got: {answer}"))?;
    assert_eq!(
        value,
        json!({ "verdict": "pass", "reason": "local path resolved" }),
        "the floor resolves the lent tree and exposes its mount path on the ToolHost"
    );

    Ok(())
}
