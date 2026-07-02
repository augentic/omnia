//! Integration test for `wasi-model` Phase 1 — the run-1 (replay) acceptance
//! gate (`rfcs/wasi-model.md` §6).
//!
//! Builds the `examples/cli-model` `create` guest, links the `WasiModel` host,
//! and drives the guest's `wasi:cli/run` export across the real WIT boundary. It
//! proves the Layer 1 invariant end-to-end:
//!
//! 1. **replay** — `ModelDefault` loaded from `examples/cli-model/fixtures` serves
//!    the recorded, validated answer for the guest with no backend at all;
//! 2. **fixture shape** — the checked-in fixture keys on the reduced prompt
//!    without leaking mount paths or non-serializable workspace handles.
//!
//! The guest component must be built first; the test skips (rather than fails)
//! when it is absent, because `cargo make ci` cleans the target directory before
//! running tests:
//!
//! ```bash
//! cargo build -p examples --example cli-model-wasm --target wasm32-wasip2
//! ```

#![cfg(not(target_arch = "wasm32"))]

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use anyhow::{Context as _, Result, bail};
use futures::FutureExt as _;
use omnia::wasmtime::StoreLimitsBuilder;
use omnia::{
    Backend, Deployment, DeploymentBuilder, GuestId, MountRegistry, Registry, ResolvedPreopen,
    Runtime, StoreBase, StoreCtx, WrpcState,
};
use omnia_wasi_model::{
    Answer, ConnectOptions, FutureResult, HasModel, ModelDefault, PreparedRequest, ToolHost,
    WasiModel, WasiModelCtx,
};
use serde_json::{Value, json};
use wasmtime_wasi::p2::pipe::MemoryOutputPipe;
use wasmtime_wasi::p3::bindings::Command;
use wasmtime_wasi::{ResourceTable, WasiCtxBuilder};

/// A factory the test bundle calls per clone to mint a fresh backend.
type BackendFactory = Arc<dyn Fn() -> Box<dyn WasiModelCtx> + Send + Sync>;

/// The deployment's backend bundle for the test: the swappable model backend the
/// test installs for replay. Its [`HasModel`] impl is what
/// `omnia::StoreCtx<TestBundle>` reads to serve `wasi-model`.
///
/// The library [`Runtime::store`] clones the bundle to build each per-guest
/// store, so the bundle's [`Clone`] mints a fresh backend (replacing the old
/// per-store factory call) and bumps `stores_built`.
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
/// mounts preopened into each store (empty when no workspace is configured, a
/// single `.` mount for the completion path so the guest lends a workspace).
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
/// via `preopens.get-directories()` and lends it through `grants.workspace`.
/// The replay key ignores the lent descriptor; any real directory serves.
fn workspace_mount() -> (PathBuf, Arc<MountRegistry>) {
    let dir = std::env::temp_dir().join(format!("omnia-model-ws-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("creating the workspace mount dir");
    let registry =
        MountRegistry::open(vec![ResolvedPreopen::new(".".to_owned(), dir.clone(), false)])
            .expect("opening the workspace mount");
    (dir, Arc::new(registry))
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
    let registry = deployment.into_registry().context("assembling registry")?;

    let _ = std::fs::remove_file(&manifest_path);
    Ok(Arc::new(registry))
}

/// Instantiate the guest fresh, drive `wasi:cli/run`, and return stdout.
async fn call_run(runtime: &Runtime<TestBundle>) -> Result<String> {
    let guest =
        runtime.registry().get(&GuestId::from("model")).context("model guest is registered")?;
    let template = runtime.store();
    let mounts = Arc::clone(&template.base.mounts);
    let stdout = MemoryOutputPipe::new(65536);
    let stdout_capture = stdout.clone();

    let mut wasi_builder = WasiCtxBuilder::new();
    wasi_builder
        .inherit_env()
        .inherit_stdin()
        .stdout(stdout)
        .stderr(tokio::io::stderr())
        .args(&["model" as &str]);
    for entry in mounts.entries() {
        let _ = wasi_builder.preopened_dir(
            &entry.host_path,
            &entry.name,
            entry.dir_perms,
            entry.file_perms,
        );
    }

    let options = runtime.options();
    let store_ctx = StoreCtx {
        base: StoreBase {
            table: ResourceTable::new(),
            wasi: wasi_builder.build(),
            limits: StoreLimitsBuilder::new().memory_size(options.max_memory_bytes).build(),
            wrpc: WrpcState::new(),
            dispatcher: Arc::clone(&template.base.dispatcher),
            mounts,
        },
        backends: template.backends.clone(),
    };

    let mut store = runtime.build_store(store_ctx);
    let instance = runtime
        .instantiate(guest.instance_pre(), &mut store)
        .await
        .context("instantiating guest")?;
    let command = Command::new(&mut store, &instance).map_err(anyhow::Error::from)?;

    let outcome = store
        .run_concurrent(async move |store| command.wasi_cli_run().call_run(store).await)
        .await
        .map_err(anyhow::Error::from)
        .context("calling wasi:cli/run")?;

    match outcome {
        Ok(Ok(())) => {}
        Ok(Err(())) => bail!("model guest returned Err from wasi:cli/run"),
        Err(error) => return Err(error.into()),
    }

    let output = stdout_capture.contents();
    String::from_utf8(output.to_vec()).context("guest stdout is utf-8")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn replays_completion_with_no_network() -> Result<()> {
    let Some(wasm) = guest_wasm(&target_dir(), "cli_model_wasm.wasm") else {
        eprintln!(
            "skipping `replays_completion_with_no_network`: cli-model guest not built. Run:\n  \
             cargo build -p examples --example cli-model-wasm --target wasm32-wasip2"
        );
        return Ok(());
    };

    let registry = registry(&wasm).await?;

    // The answer the recorded run produces and the replay must reproduce.
    let expected = expected_answer();

    // The completion path preopens a workspace the example guest lends; the host
    // resolves the lent descriptor back to this mount by identity.
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
    assert!(
        !recorded.contains(mount_dir.to_string_lossy().as_ref()),
        "the fixture key must not leak the mount's host path"
    );

    let replayed = replay_from(&registry, &fixtures, &mounts)
        .await
        .context("replay from committed fixture")?;
    let parsed: Value = serde_json::from_str(&replayed)
        .with_context(|| format!("replayed answer should be JSON, got: {replayed}"))?;
    assert_eq!(parsed, expected, "checked-in example fixture should replay the guest");

    Ok(())
}

/// The answer the example guest's prompt resolves to — the value every replay must
/// reproduce.
fn expected_answer() -> Value {
    json!({ "verdict": "pass", "reason": "the bounds check is correct" })
}

/// The checked-in example fixture directory (`examples/cli-model/fixtures`).
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

/// A backend that asserts the host resolved the guest's lent workspace to
/// its mount path — the `local-path` face the cursor backend consumes.
#[derive(Debug, Clone)]
struct LocalPathProbe {
    expected: PathBuf,
}

impl WasiModelCtx for LocalPathProbe {
    fn complete(
        &self, _request: PreparedRequest, tool_host: Arc<dyn ToolHost>,
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
                usage: None,
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
    let Some(wasm) = guest_wasm(&target_dir(), "cli_model_wasm.wasm") else {
        eprintln!(
            "skipping `workspace_resolves_to_local_path`: cli-model guest not built. Run:\n  \
             cargo build -p examples --example cli-model-wasm --target wasm32-wasip2"
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
