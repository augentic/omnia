//! `wasi-model` acceptance gate.
//!
//! Builds the `examples/model` guest, links the `WasiModel` host, and drives
//! the guest's `wasi:cli/run` export across the real WIT boundary. It proves
//! the Layer 1 invariant end-to-end:
//!
//! 1. **canned** — an inline `Canned` backend serves a fixed, validated
//!    answer for the guest with no live model at all;
//! 2. **echo default** — `ModelDefault` connects with zero configuration and
//!    echoes text/json prompts, but rejects `format::schema` (the example
//!    guest's format) since no echo can conform to a guest schema.
//!
//! The registry (component + linker + `InstancePre`) is built once and shared
//! by all tests; each test assembles its own runtime over it.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context as _, Result, bail};
use futures::FutureExt as _;
use omnia::wasmtime::StoreLimitsBuilder;
use omnia::{
    Backend, Deployment, DeploymentBuilder, GuestId, Manifest, MountRegistry, Registry,
    ResolvedPreopen, Runtime, StoreBase, StoreCtx, WrpcState,
};
use omnia_testkit::{find_guest, temp_manifest};
use omnia_wasi_model::{
    Answer, FutureResult, HasModel, ModelDefault, Request, SessionLimits, Tool, ToolHost,
    WasiModel, WasiModelCtx,
};
use serde_json::{Value, json};
use tokio::sync::OnceCell;
use wasmtime_wasi::p2::pipe::MemoryOutputPipe;
use wasmtime_wasi::p3::bindings::Command;
use wasmtime_wasi::{ResourceTable, WasiCtxBuilder};

use crate::fixture;

/// A factory the test bundle calls per clone to mint a fresh backend.
type BackendFactory = Arc<dyn Fn() -> Box<dyn WasiModelCtx> + Send + Sync>;

/// The deployment's backend bundle for the test: the swappable model backend
/// each test installs. Its [`HasModel`] impl is what
/// `omnia::StoreCtx<TestBundle>` reads to serve `wasi-model`.
///
/// The library [`Runtime::store`] clones the bundle to build each per-guest
/// store, so the bundle's [`Clone`] mints a fresh backend (replacing the old
/// per-store factory call).
struct TestBundle {
    backend: BackendFactory,
    model: Box<dyn WasiModelCtx>,
}

impl Clone for TestBundle {
    fn clone(&self) -> Self {
        Self {
            backend: Arc::clone(&self.backend),
            model: (self.backend)(),
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
    registry: Arc<Registry<TestCtx>>, backend: BackendFactory, mounts: Arc<MountRegistry>,
) -> Runtime<TestBundle> {
    let bundle = TestBundle {
        model: backend(),
        backend,
    };
    Runtime::from_parts(registry, Vec::new(), mounts, bundle)
}

/// A single read-only workspace mount named `.` over a fresh temp directory —
/// the shape `omnia.toml`'s `[[mount]]` resolves to. The example guest reads it
/// via `preopens.get-directories()` and lends it through `grants.workspace`.
/// The canned backend ignores the request; any real directory serves.
fn workspace_mount() -> (PathBuf, Arc<MountRegistry>) {
    workspace_mount_at("ws")
}

/// [`workspace_mount`] over a `label`-specific directory, so a test needing
/// two distinct trees does not collide with the shared one.
fn workspace_mount_at(label: &str) -> (PathBuf, Arc<MountRegistry>) {
    let dir = std::env::temp_dir().join(format!("omnia-model-{label}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("creating the workspace mount dir");
    let registry =
        MountRegistry::open(vec![ResolvedPreopen::new(".".to_owned(), dir.clone(), false)])
            .expect("opening the workspace mount");
    (dir, Arc::new(registry))
}

/// The shared model registry: the guest deployment with `WasiModel` linked,
/// built once for both tests.
async fn registry() -> Result<&'static Arc<Registry<TestCtx>>> {
    static CELL: OnceCell<Arc<Registry<TestCtx>>> = OnceCell::const_new();
    CELL.get_or_try_init(build_registry).await
}

async fn build_registry() -> Result<Arc<Registry<TestCtx>>> {
    let wasm = find_guest("model_wasm.wasm");

    // A one-guest manifest with an absolute source path. The guard removes the
    // temp file when it drops at the end of this function — after `build` has
    // read it.
    let manifest = temp_manifest(&format!(
        "[[guest]]\nid = \"model\"\nsource.path = \"{}\"\n",
        wasm.display()
    ))?;

    let builder =
        DeploymentBuilder::new().manifest(Manifest::from_config(manifest.path())?).precompiled();
    // SAFETY: `find_guest` only returns artifacts this workspace built and
    // serialized itself (`cargo make test-guests`).
    let mut deployment: Deployment<TestCtx> =
        unsafe { builder.build() }.await.context("building runtime")?;
    deployment.host::<WasiModel, TestBundle>().context("linking WasiModel")?;
    let registry = deployment.into_registry().context("assembling registry")?;

    Ok(Arc::new(registry))
}

/// Instantiate the guest fresh, drive `wasi:cli/run`, and return stdout.
async fn call_run(runtime: &Runtime<TestBundle>) -> Result<String> {
    call_scenario(runtime, "default").await
}

/// [`call_run`] selecting a guest scenario via its CLI argument.
async fn call_scenario(runtime: &Runtime<TestBundle>, scenario: &str) -> Result<String> {
    call_run_with(runtime, scenario, None).await
}

/// [`call_run`] preopening `preopens` into the guest instead of the runtime's
/// own mounts — the guest then lends a directory the deployment never
/// authorized (host-side identity matching still runs against the runtime's
/// registry).
async fn call_run_preopening(
    runtime: &Runtime<TestBundle>, preopens: Option<Arc<MountRegistry>>,
) -> Result<String> {
    call_run_with(runtime, "default", preopens).await
}

async fn call_run_with(
    runtime: &Runtime<TestBundle>, scenario: &str, preopens: Option<Arc<MountRegistry>>,
) -> Result<String> {
    let guest =
        runtime.registry().get(&GuestId::from("model")).context("model guest is registered")?;
    let template = runtime.store();
    let mounts = Arc::clone(&template.base.mounts);
    let preopens = preopens.unwrap_or_else(|| Arc::clone(&mounts));
    let stdout = MemoryOutputPipe::new(65536);
    let stdout_capture = stdout.clone();

    let mut wasi_builder = WasiCtxBuilder::new();
    wasi_builder
        .inherit_env()
        .inherit_stdin()
        .stdout(stdout)
        .stderr(tokio::io::stderr())
        .args(&["model", scenario]);
    for entry in preopens.entries() {
        wasi_builder
            .preopened_dir(&entry.host_path, &entry.name, entry.dir_perms, entry.file_perms)
            .map_err(|error| {
                anyhow::anyhow!("preopening `{}`: {error}", entry.host_path.display())
            })?;
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

/// A backend that answers every completion with one fixed JSON value.
#[derive(Clone, Debug)]
struct Canned(Value);

impl WasiModelCtx for Canned {
    fn complete(&self, _request: Request, _tool_host: Arc<dyn ToolHost>) -> FutureResult<Answer> {
        let answer = Answer {
            value: self.0.clone(),
            usage: None,
            transcript: None,
        };
        async move { Ok(answer) }.boxed()
    }
}

// The canned backend serves a fixed answer, so the completion round-trips
// with no network.
#[test]
fn canned() -> Result<()> {
    fixture::RT.block_on(async {
        let registry = registry().await?;

        // The answer the guest must print after the host validates it.
        let expected = answer();

        // The completion path preopens a workspace the example guest lends; the host
        // resolves the lent descriptor back to this mount by identity.
        let (_mount_dir, mounts) = workspace_mount();

        let backend = Canned(expected.clone());
        let runtime = model_runtime(
            Arc::clone(registry),
            Arc::new(move || Box::new(backend.clone())),
            mounts,
        );

        let answer = call_run(&runtime).await.context("driving the canned backend")?;
        let parsed: Value = serde_json::from_str(&answer)
            .with_context(|| format!("answer should be JSON, got: {answer}"))?;
        assert_eq!(parsed, expected, "the canned answer round-trips to the guest");

        Ok(())
    })
}

/// The answer the canned backend serves — the value the guest must print.
fn answer() -> Value {
    json!({ "verdict": "pass", "reason": "the bounds check is correct" })
}

// The echo default under a schema-format guest: `ModelDefault` connects with
// zero configuration, but the example guest asks for `format::schema`, which
// an echo cannot satisfy — the completion fails with a backend error naming
// the gap.
#[test]
fn rejects_schema() -> Result<()> {
    fixture::RT.block_on(async {
        let registry = registry().await?;
        let (_mount_dir, mounts) = workspace_mount();
        let backend = ModelDefault::connect().await.context("connecting the default backend")?;
        let runtime =
            model_runtime(Arc::clone(registry), Arc::new(move || Box::new(backend)), mounts);

        let output = call_run(&runtime).await.context("driving the default backend")?;
        assert!(
            output.contains("cannot satisfy format::schema"),
            "the echo default must reject schema formats, got: {output}"
        );

        Ok(())
    })
}

/// A backend that asserts the host resolved the guest's lent workspace to
/// its mount path — the `local-path` face the cursor backend consumes — and
/// that the bounded `read` face serves the mount's contents.
#[derive(Debug, Clone)]
struct PathProbe {
    expected: PathBuf,
}

impl WasiModelCtx for PathProbe {
    fn complete(&self, _request: Request, tool_host: Arc<dyn ToolHost>) -> FutureResult<Answer> {
        let expected = self.expected.clone();
        async move {
            let local = tool_host.local_path().map(Path::to_path_buf);
            anyhow::ensure!(
                local.as_deref() == Some(expected.as_path()),
                "host must resolve the lent workspace to its mount path: got {local:?}, want {}",
                expected.display()
            );
            let bytes = tool_host.read("hello.txt".to_owned()).await?;
            anyhow::ensure!(bytes == b"hi", "workspace read returns the seeded file's bytes");
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
#[test]
fn workspace() -> Result<()> {
    fixture::RT.block_on(async {
        let registry = registry().await?;
        let (mount_dir, mounts) = workspace_mount_at("probe");
        std::fs::write(mount_dir.join("hello.txt"), b"hi").context("seeding the mount")?;
        let expected = mount_dir.clone();
        let runtime = model_runtime(
            Arc::clone(registry),
            Arc::new(move || {
                Box::new(PathProbe {
                    expected: expected.clone(),
                })
            }),
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
    })
}

// The guest lends a directory the deployment never authorized: the host's
// identity match rejects it before the backend ever runs.
#[test]
fn out_of_scope_workspace() -> Result<()> {
    fixture::RT.block_on(async {
        let registry = registry().await?;
        // The runtime authorizes one tree, but the guest is preopened (and so
        // lends) a different one.
        let (_authorized, mounts) = workspace_mount_at("scope-ok");
        let (_lent, lent) = workspace_mount_at("scope-bad");

        let backend = Canned(answer());
        let runtime = model_runtime(
            Arc::clone(registry),
            Arc::new(move || Box::new(backend.clone())),
            mounts,
        );

        let output = call_run_preopening(&runtime, Some(lent))
            .await
            .context("driving the out-of-scope lend")?;
        assert!(
            output.contains("out of scope"),
            "an unauthorized lend is rejected in the host: {output}"
        );

        Ok(())
    })
}

/// A backend that drives `ToolHost::write` against the lent workspace and
/// reports the outcome as its answer.
#[derive(Debug, Clone)]
struct WriteProbe;

impl WasiModelCtx for WriteProbe {
    fn complete(&self, _request: Request, tool_host: Arc<dyn ToolHost>) -> FutureResult<Answer> {
        async move {
            let reason = match tool_host.write("probe.txt".to_owned(), b"data".to_vec()).await {
                Ok(()) => "write unexpectedly succeeded".to_owned(),
                Err(error) => format!("{error:#}"),
            };
            Ok(Answer {
                value: json!({ "verdict": "probe", "reason": reason }),
                usage: None,
                transcript: None,
            })
        }
        .boxed()
    }
}

// A write through the ToolHost against a read-only mount is denied in the
// host, and nothing lands on disk.
#[test]
fn readonly_write_denied() -> Result<()> {
    fixture::RT.block_on(async {
        let registry = registry().await?;
        let (mount_dir, mounts) = workspace_mount_at("ro");
        let runtime =
            model_runtime(Arc::clone(registry), Arc::new(move || Box::new(WriteProbe)), mounts);

        let output = call_run(&runtime).await.context("driving the write probe")?;
        let value: Value = serde_json::from_str(&output)
            .with_context(|| format!("probe answer should be JSON, got: {output}"))?;
        assert!(
            value["reason"].as_str().is_some_and(|reason| reason.contains("read-only")),
            "the denial names the read-only mount: {value}"
        );
        assert!(!mount_dir.join("probe.txt").exists(), "no file lands on a denied write");

        Ok(())
    })
}

// -- Tool-session scenarios: probe backends drive `ToolHost::call_tool`
// -- against the scenario-selected guest.

type Events = Arc<std::sync::Mutex<Vec<String>>>;

fn push(events: &Events, event: String) {
    events.lock().unwrap_or_else(std::sync::PoisonError::into_inner).push(event);
}

fn drain(events: &Events) -> Vec<String> {
    events.lock().unwrap_or_else(std::sync::PoisonError::into_inner).clone()
}

/// Build a `model_runtime` over a probe backend with a plain workspace mount
/// (the tool scenarios lend nothing; the mount only satisfies the fixture).
fn probe_runtime<P>(registry: &Arc<Registry<TestCtx>>, label: &str, probe: P) -> Runtime<TestBundle>
where
    P: WasiModelCtx + Clone,
{
    let (_dir, mounts) = workspace_mount_at(label);
    model_runtime(Arc::clone(registry), Arc::new(move || Box::new(probe.clone())), mounts)
}

/// One `call_tool` round trip: the guest's `complete_with` closure answers
/// over its own locals and the reply embeds the result.
#[derive(Debug, Clone)]
struct ToolOnceProbe;

impl WasiModelCtx for ToolOnceProbe {
    fn complete(&self, request: Request, tool_host: Arc<dyn ToolHost>) -> FutureResult<Answer> {
        async move {
            anyhow::ensure!(
                request.tools.iter().any(|tool| matches!(
                    tool,
                    Tool::Function(function) if function.name == "lookup"
                )),
                "the request advertises the guest's declared function tool"
            );
            let value = tool_host
                .call_tool("lookup".to_owned(), "k1".to_owned())
                .await?
                .map_err(|failure| anyhow::anyhow!("tool failed: {failure}"))?;
            Ok(Answer {
                value: json!({ "verdict": "pass", "tool": value }),
                usage: None,
                transcript: None,
            })
        }
        .boxed()
    }
}

#[test]
fn tool_round_trip() -> Result<()> {
    fixture::RT.block_on(async {
        let registry = registry().await?;
        let runtime = probe_runtime(registry, "tool-once", ToolOnceProbe);

        let output = call_scenario(&runtime, "round_trip").await?;
        let value: Value = serde_json::from_str(&output)
            .with_context(|| format!("reply should be JSON, got: {output}"))?;
        assert_eq!(
            value,
            json!({ "verdict": "pass", "tool": "v1" }),
            "the guest closure's shelf value rides back through the reply"
        );

        Ok(())
    })
}

/// Three concurrent calls; the guest batches them and answers in reverse, so
/// results correlate by id, not order.
#[derive(Debug, Clone)]
struct ParallelProbe;

impl WasiModelCtx for ParallelProbe {
    fn complete(&self, _request: Request, tool_host: Arc<dyn ToolHost>) -> FutureResult<Answer> {
        async move {
            let (a, b, c) = futures::join!(
                tool_host.call_tool("echo".to_owned(), "a".to_owned()),
                tool_host.call_tool("echo".to_owned(), "b".to_owned()),
                tool_host.call_tool("echo".to_owned(), "c".to_owned()),
            );
            for (argument, outcome) in [("a", a), ("b", b), ("c", c)] {
                let value = outcome?.map_err(|failure| anyhow::anyhow!("tool: {failure}"))?;
                anyhow::ensure!(
                    value == format!("echo:{argument}"),
                    "result for `{argument}` correlates by id, got `{value}`"
                );
            }
            Ok(Answer {
                value: json!({ "verdict": "parallel" }),
                usage: None,
                transcript: None,
            })
        }
        .boxed()
    }
}

#[test]
fn parallel_calls() -> Result<()> {
    fixture::RT.block_on(async {
        let registry = registry().await?;
        let runtime = probe_runtime(registry, "parallel", ParallelProbe);

        let output = call_scenario(&runtime, "parallel").await?;
        assert_eq!(
            output.matches("answered:").count(),
            3,
            "the guest answers all three batched calls: {output}"
        );
        assert!(
            output.contains("calls-closed") && output.contains("reply-ok:"),
            "reversed answers still satisfy the probe: {output}"
        );

        Ok(())
    })
}

/// The guest drops its calls reader and reply future mid-loop. The probe's
/// in-flight work is structurally cancelled (or its next call hard-fails,
/// whichever the guest's drops reach first) and the session shuts down — the
/// guest observes the end via a rejected results write, so nothing hangs.
#[derive(Debug, Clone)]
struct DropProbe {
    events: Events,
}

/// Logs `cancelled` if the probe future is dropped before completing.
struct CancelLog {
    events: Events,
    armed: bool,
}

impl Drop for CancelLog {
    fn drop(&mut self) {
        if self.armed {
            push(&self.events, "cancelled".to_owned());
        }
    }
}

impl WasiModelCtx for DropProbe {
    fn complete(&self, _request: Request, tool_host: Arc<dyn ToolHost>) -> FutureResult<Answer> {
        let events = Arc::clone(&self.events);
        async move {
            let mut guard = CancelLog {
                events: Arc::clone(&events),
                armed: true,
            };
            let first = tool_host.call_tool("echo".to_owned(), "one".to_owned()).await;
            push(
                &events,
                match &first {
                    Ok(Ok(value)) => format!("first-ok:{value}"),
                    other => format!("first-unexpected:{other:?}"),
                },
            );
            let second = tool_host.call_tool("echo".to_owned(), "two".to_owned()).await;
            guard.armed = false;
            match second {
                Err(error) => {
                    push(&events, format!("second-err:{error:#}"));
                    Err(error)
                }
                Ok(inner) => {
                    push(&events, format!("second-ok:{inner:?}"));
                    anyhow::bail!("the dropped session must fail the second call")
                }
            }
        }
        .boxed()
    }
}

#[test]
fn guest_drops_session() -> Result<()> {
    fixture::RT.block_on(async {
        let registry = registry().await?;
        let events: Events = Arc::default();
        let probe = DropProbe {
            events: Arc::clone(&events),
        };
        let runtime = probe_runtime(registry, "drops", probe);

        let output = call_scenario(&runtime, "drops_session").await?;
        assert!(
            output.contains("dropped-session") && output.contains("host-acked-via-results-reject"),
            "the guest observes the session's end after dropping its ends: {output}"
        );

        let events = drain(&events);
        assert_eq!(
            events.first().map(String::as_str),
            Some("first-ok:echo:one"),
            "the first call completes before the drop: {events:?}"
        );
        assert!(
            events.iter().any(|event| event.starts_with("second-err:") || event == "cancelled"),
            "the drop fails the next call or cancels the backend — no leaked waiter: {events:?}"
        );

        Ok(())
    })
}

/// The guest closes its results stream with a call pending: the call
/// hard-fails and the reply still resolves with a typed error.
#[derive(Debug, Clone)]
struct PendingCallProbe;

impl WasiModelCtx for PendingCallProbe {
    fn complete(&self, _request: Request, tool_host: Arc<dyn ToolHost>) -> FutureResult<Answer> {
        async move {
            match tool_host.call_tool("echo".to_owned(), "one".to_owned()).await {
                Err(error) => Err(error),
                Ok(inner) => anyhow::bail!("the pending call must hard-fail, got {inner:?}"),
            }
        }
        .boxed()
    }
}

#[test]
fn guest_closes_results_early() -> Result<()> {
    fixture::RT.block_on(async {
        let registry = registry().await?;
        let runtime = probe_runtime(registry, "closes", PendingCallProbe);

        let output = call_scenario(&runtime, "closes_results").await?;
        assert!(
            output.contains("results-writer-dropped") && output.contains("calls-closed"),
            "the calls stream closes after the guest drops its writer: {output}"
        );
        assert!(
            output.contains("reply-err:") && output.contains("closed its results stream"),
            "the reply resolves with a typed error naming the closed stream: {output}"
        );

        Ok(())
    })
}

/// Loops `call_tool` until the host's per-completion budget trips.
#[derive(Debug, Clone)]
struct BudgetProbe;

impl WasiModelCtx for BudgetProbe {
    fn complete(&self, _request: Request, tool_host: Arc<dyn ToolHost>) -> FutureResult<Answer> {
        async move {
            for turn in 0..16 {
                tool_host
                    .call_tool("echo".to_owned(), turn.to_string())
                    .await?
                    .map_err(|failure| anyhow::anyhow!("tool: {failure}"))?;
            }
            anyhow::bail!("the budget must trip before 16 calls")
        }
        .boxed()
    }

    fn limits(&self) -> SessionLimits {
        SessionLimits {
            max_tool_calls: 2,
            ..SessionLimits::default()
        }
    }
}

#[test]
fn budget_exhausted() -> Result<()> {
    fixture::RT.block_on(async {
        let registry = registry().await?;
        let runtime = probe_runtime(registry, "budget", BudgetProbe);

        let output = call_scenario(&runtime, "budget").await?;
        assert!(
            output.contains("BudgetExhausted") && output.contains("budget of 2 exhausted"),
            "the reply carries the typed budget failure: {output}"
        );

        Ok(())
    })
}

/// One call whose result blows the byte cap: the typed failure wins over the
/// tool's own output.
#[derive(Debug, Clone)]
struct OversizeProbe;

impl WasiModelCtx for OversizeProbe {
    fn complete(&self, _request: Request, tool_host: Arc<dyn ToolHost>) -> FutureResult<Answer> {
        async move {
            match tool_host.call_tool("blob".to_owned(), "{}".to_owned()).await {
                Err(error) => Err(error),
                Ok(inner) => anyhow::bail!("the oversize result must hard-fail, got {inner:?}"),
            }
        }
        .boxed()
    }

    fn limits(&self) -> SessionLimits {
        SessionLimits {
            max_result_bytes: 256,
            ..SessionLimits::default()
        }
    }
}

#[test]
fn oversize_result() -> Result<()> {
    fixture::RT.block_on(async {
        let registry = registry().await?;
        let runtime = probe_runtime(registry, "oversize", OversizeProbe);

        let output = call_scenario(&runtime, "oversize").await?;
        assert!(
            output.contains("ToolFailed") && output.contains("exceeds the 256-byte cap"),
            "the reply carries the typed size-cap failure: {output}"
        );

        Ok(())
    })
}

/// A call the guest never answers: the host's per-call timeout ends the
/// session with a typed error instead of hanging the completion.
#[derive(Debug, Clone)]
struct StallProbe;

impl WasiModelCtx for StallProbe {
    fn complete(&self, _request: Request, tool_host: Arc<dyn ToolHost>) -> FutureResult<Answer> {
        async move {
            match tool_host.call_tool("echo".to_owned(), "one".to_owned()).await {
                Err(error) => Err(error),
                Ok(inner) => anyhow::bail!("the stalled call must time out, got {inner:?}"),
            }
        }
        .boxed()
    }

    fn limits(&self) -> SessionLimits {
        SessionLimits {
            tool_timeout: std::time::Duration::from_millis(250),
            ..SessionLimits::default()
        }
    }
}

#[test]
fn stalled_handler() -> Result<()> {
    fixture::RT.block_on(async {
        let registry = registry().await?;
        let runtime = probe_runtime(registry, "stall", StallProbe);

        let output = call_scenario(&runtime, "stall").await?;
        assert!(
            output.contains("received:") && output.contains("got no result within"),
            "the reply carries the typed timeout failure: {output}"
        );

        Ok(())
    })
}
