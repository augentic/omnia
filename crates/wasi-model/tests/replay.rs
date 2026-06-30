//! Integration test for `wasi-model` Phase 1 — the run-1 (replay) acceptance
//! gate (`rfcs/wasi-model.md` §6).
//!
//! Builds the `examples/model` `complete` guest, links the `WasiModel` host, and
//! drives the guest's `run` export across the real WIT boundary. It proves the
//! Layer 1 invariant end-to-end:
//!
//! 1. **replay** — `ModelDefault` loaded from `examples/model/fixtures` serves the
//!    recorded, validated answer for the guest with no backend at all;
//! 2. **fixture shape** — the checked-in fixture keys on `workspace_lent = true`
//!    without leaking the mount's host path.
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
    Answer, ConnectOptions, FutureResult, HasModel, ModelDefault, PreparedPrompt, Reference,
    ToolHost, WasiModel, WasiModelCtx,
};
use serde_json::{Value, json};

/// A factory the test bundle calls per clone to mint a fresh backend.
type BackendFactory = Arc<dyn Fn() -> Box<dyn WasiModelCtx> + Send + Sync>;

/// The deployment's backend bundle for the test: the swappable model backend the
/// test installs for replay. Its [`HasModel`] impl is what
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
/// backend `factory` (cloned fresh into every store) and the `mounts`
/// mounts preopened into each store (empty for the `resolve` path, a single `.`
/// mount for the completion path so the guest lends a workspace). The shared
/// shared `stores_built` counts bundle clones — the instance-creation witness.
fn model_runtime(
    registry: Arc<Registry<TestCtx>>, backend: BackendFactory, stores_built: Arc<AtomicUsize>,
    mounts: Arc<MountRegistry>,
) -> Runtime<TestBundle> {
    let bundle = TestBundle {
        model: backend(),
        backend,
        stores_built,
    };
    Runtime::from_parts(registry, Vec::new(), mounts, bundle)
}

/// A single read-only workspace mount named `.` over a fresh temp directory —
/// the shape `omnia.toml`'s `[[mount]]` resolves to. The example guest reads it
/// via `preopens.get-directories()` and lends it through `grants.workspace`,
/// so the recorded prompt carries `workspace_lent = true`. The directory's
/// identity is irrelevant to the replay key (only the boolean marker lands
/// there), so any real directory serves.
fn workspace_mount() -> (PathBuf, Arc<MountRegistry>) {
    let dir = std::env::temp_dir().join(format!("omnia-model-ws-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("creating the workspace mount dir");
    let registry =
        MountRegistry::open(vec![ResolvedPreopen::new(".".to_owned(), dir.clone(), false)])
            .expect("opening the workspace mount");
    (dir, Arc::new(registry))
}

/// An empty registry — no preopens, the default for paths that don't exercise a
/// workspace.
fn no_mounts() -> Arc<MountRegistry> {
    Arc::new(MountRegistry::default())
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

    // The completion path preopens a workspace the example guest lends, so the
    // fixture key carries `workspace_lent = true` and the host resolves the
    // lent descriptor back to this mount by identity.
    let (mount_dir, mounts) = workspace_mount();

    let fixtures = committed_fixtures();
    let fixture_files: Vec<PathBuf> = std::fs::read_dir(&fixtures)
        .context("reading fixture dir")?
        .filter_map(std::result::Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.extension().and_then(|e| e.to_str()) == Some("json"))
        .collect();
    assert_eq!(fixture_files.len(), 1, "expected exactly one checked-in fixture");

    let recorded = std::fs::read_to_string(&fixture_files[0]).context("reading fixture")?;
    let fixture: Value = serde_json::from_str(&recorded).context("fixture is JSON")?;
    assert_eq!(
        fixture["key_prompt"]["grants"]["workspace_lent"],
        json!(true),
        "a lent workspace keys as `workspace_lent: true`"
    );
    assert!(
        !recorded.contains(mount_dir.to_string_lossy().as_ref()),
        "the fixture key must not leak the mount's host path"
    );

    let replayed = replay_from(&registry, &fixtures, &mounts)
        .await
        .context("replay from committed fixture")?;
    assert_eq!(
        serde_json::from_str::<Value>(&replayed).context("answer is JSON")?,
        expected,
        "checked-in example fixture should replay the guest"
    );

    Ok(())
}

/// The answer the example guest's prompt resolves to — the value every replay must
/// reproduce.
fn expected_answer() -> Value {
    json!({ "verdict": "pass", "reason": "the bounds check is correct" })
}

/// The checked-in example fixture directory (`examples/model/fixtures`).
fn committed_fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/model/fixtures")
}

/// Replay the guest with a `ModelDefault` backend loaded from `dir`.
async fn replay_from(
    registry: &Arc<Registry<TestCtx>>, dir: &Path, mounts: &Arc<MountRegistry>,
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
        Arc::clone(mounts),
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
        &self, _request: PreparedPrompt, tool_host: Arc<dyn ToolHost>,
    ) -> FutureResult<Answer> {
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
            Ok(Answer {
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
        // The `resolve` path doesn't exercise a workspace; the model guest's
        // `get-directories()` sees no mount and lends nothing.
        no_mounts(),
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

/// A backend that asserts the host resolved the guest's lent workspace to
/// its mount path — the `local-path` face the cursor backend consumes.
#[derive(Debug, Clone)]
struct LocalPathProbe {
    expected: PathBuf,
}

impl WasiModelCtx for LocalPathProbe {
    fn complete(
        &self, _request: PreparedPrompt, tool_host: Arc<dyn ToolHost>,
    ) -> FutureResult<Answer> {
        let expected = self.expected.clone();
        async move {
            let local = tool_host.local_path().map(Path::to_path_buf);
            anyhow::ensure!(
                local.as_deref() == Some(expected.as_path()),
                "host must resolve the lent workspace to its mount path: got {local:?}, want {}",
                expected.display()
            );
            Ok(Answer {
                value: json!({ "verdict": "pass", "reason": "local path resolved" }),
                transcript: None,
            })
        }
        .boxed()
    }
}

/// The workspace `local-path` face end-to-end: the host preopens a `.` mount,
/// the example guest reads it via `preopens.get-directories()` and lends it, and
/// the host identity-matches it back to the mount — surfacing its host path on
/// the per-completion [`ToolHost`] (what `omnia-cursor` reads).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn workspace_resolves_to_local_path() -> Result<()> {
    let Some(wasm) = guest_wasm(&target_dir(), "model_wasm.wasm") else {
        eprintln!(
            "skipping `workspace_resolves_to_local_path`: model guest not built. Run:\n  \
             cargo build -p examples --example model-wasm --target wasm32-wasip2"
        );
        return Ok(());
    };

    let registry = registry(&wasm).await?;
    let (mount_dir, mounts) = workspace_mount();
    let expected = mount_dir.clone();
    let runtime = model_runtime(
        registry,
        Arc::new(move || {
            Box::new(LocalPathProbe {
                expected: expected.clone(),
            })
        }),
        Arc::new(AtomicUsize::new(0)),
        mounts,
    );

    let answer = call_run(&runtime).await.context("driving the local-path probe")?;
    let value: Value = serde_json::from_str(&answer)
        .with_context(|| format!("probe answer should be JSON, got: {answer}"))?;
    assert_eq!(
        value,
        json!({ "verdict": "pass", "reason": "local path resolved" }),
        "the host resolves the lent workspace and exposes its mount path on the ToolHost"
    );

    Ok(())
}
