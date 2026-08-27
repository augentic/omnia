//! End-to-end tests for `omnia:model/completion`: every scenario runs a real
//! guest component from `crates/test-programs` through the omnia runtime
//! against an inline scenario backend. The guest asserts what it observes
//! across the boundary (and traps on failure); the host side asserts wire
//! fidelity and filesystem effects.

#![cfg(not(target_arch = "wasm32"))]

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures::FutureExt as _;
use omnia::{Deployment, ExitStatus, Mount, StoreCtx};
use omnia_wasi_model::{
    Answer, FutureResult, HasModel, Limits, ModelDefault, Request, ToolHost, WasiModel,
    WasiModelCtx,
};
use serde_json::{Value, json};

// Every guest program in `crates/test-programs` must have a matching test
// here; a new program without one fails to compile.
macro_rules! assert_test_exists {
    ($name:ident) => {
        #[expect(unused_imports, reason = "asserts the test exists")]
        use self::$name as _;
    };
}
test_utils::foreach_model!(assert_test_exists);

// ------------------------------------------------------------------------
// Harness
// ------------------------------------------------------------------------

/// The store's backend bundle: just the model backend under test.
#[derive(Clone, Debug)]
struct Scenario<M>(M);

impl<M: WasiModelCtx + Clone> HasModel for Scenario<M> {
    fn model_ctx(&mut self) -> &mut dyn WasiModelCtx {
        &mut self.0
    }
}

async fn run_guest<M: WasiModelCtx + Clone>(
    wasm: &str, mounts: Vec<Mount>, model: M,
) -> anyhow::Result<ExitStatus> {
    test_utils::run_command(
        wasm,
        mounts,
        Scenario(model),
        |deployment: &mut Deployment<StoreCtx<Scenario<M>>>| {
            deployment.host::<WasiModel, Scenario<M>>()?;
            Ok(())
        },
    )
    .await
}

/// A fresh scratch directory mounted into the guest as `.`.
fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("omnia_model_{tag}_{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("creating scratch dir");
    dir
}

fn mount(path: &Path, writable: bool) -> Mount {
    Mount {
        name: ".".to_owned(),
        path: path.to_path_buf(),
        writable,
    }
}

// ------------------------------------------------------------------------
// Scenario backends
// ------------------------------------------------------------------------

/// Fails the test if the backend is ever reached.
#[derive(Clone, Copy, Debug)]
struct Unreached;

impl WasiModelCtx for Unreached {
    fn complete(&self, request: Request, _tool_host: Arc<dyn ToolHost>) -> FutureResult<Answer> {
        panic!("backend must not be reached: {request:?}");
    }
}

/// Answers every completion with a fixed value.
#[derive(Clone, Debug)]
struct Canned(Value);

impl WasiModelCtx for Canned {
    fn complete(&self, _request: Request, _tool_host: Arc<dyn ToolHost>) -> FutureResult<Answer> {
        let value = self.0.clone();
        async move { Ok(answer(value)) }.boxed()
    }
}

/// Answers with a value keyed off the request's last message text.
#[derive(Clone, Copy, Debug)]
struct Scripted(fn(&str) -> Value);

impl WasiModelCtx for Scripted {
    fn complete(&self, request: Request, _tool_host: Arc<dyn ToolHost>) -> FutureResult<Answer> {
        let marker =
            request.messages.last().map(|message| message.content.clone()).unwrap_or_default();
        let value = (self.0)(&marker);
        async move { Ok(answer(value)) }.boxed()
    }
}

/// Calls one declared tool `calls` times in sequence and answers with the
/// last outcome (a failure becomes model-visible repair text).
#[derive(Clone, Debug)]
struct ToolDriver {
    tool: &'static str,
    calls: u32,
    limits: Limits,
}

impl WasiModelCtx for ToolDriver {
    fn complete(&self, _request: Request, tool_host: Arc<dyn ToolHost>) -> FutureResult<Answer> {
        let (tool, calls) = (self.tool.to_owned(), self.calls);
        async move {
            let mut last = String::new();
            for _ in 0..calls {
                last = match tool_host.call_tool(tool.clone(), "{}".to_owned()).await? {
                    Ok(output) => output,
                    Err(failure) => format!("tool failed: {failure}"),
                };
            }
            Ok(answer(Value::String(last)))
        }
        .boxed()
    }

    fn limits(&self) -> Limits {
        self.limits
    }
}

/// Issues two `lookup` calls concurrently and answers with both outputs in
/// issue order, proving results correlate by id however the guest answers.
#[derive(Clone, Copy, Debug)]
struct ParallelLookups;

impl WasiModelCtx for ParallelLookups {
    fn complete(&self, _request: Request, tool_host: Arc<dyn ToolHost>) -> FutureResult<Answer> {
        async move {
            let (first, second) = futures::join!(
                tool_host.call_tool("lookup".to_owned(), "1".to_owned()),
                tool_host.call_tool("lookup".to_owned(), "2".to_owned()),
            );
            let first = first?.map_err(|failure| anyhow::anyhow!("first call: {failure}"))?;
            let second = second?.map_err(|failure| anyhow::anyhow!("second call: {failure}"))?;
            Ok(answer(Value::String(format!("{first}|{second}"))))
        }
        .boxed()
    }
}

/// Drives the host-injected workspace tools: reads the seed file, writes a
/// new one, and answers with the seed content plus the sorted listing.
#[derive(Clone, Copy, Debug)]
struct WorkspaceDriver;

impl WasiModelCtx for WorkspaceDriver {
    fn complete(&self, _request: Request, tool_host: Arc<dyn ToolHost>) -> FutureResult<Answer> {
        async move {
            let seed = tool_host.read("seed.txt".to_owned()).await?;
            tool_host.write("out.txt".to_owned(), b"written".to_vec()).await?;
            let mut names: Vec<String> =
                tool_host.list(String::new()).await?.into_iter().map(|entry| entry.name).collect();
            names.sort();
            let text = format!("{}:{}", String::from_utf8_lossy(&seed), names.join(","));
            Ok(answer(Value::String(text)))
        }
        .boxed()
    }
}

/// The wire-fidelity snapshot [`Recording`] captures per request (the full
/// `Request` is not `Clone`: its workspace grant holds a resource handle).
#[derive(Clone, Debug)]
struct Seen {
    system: Option<String>,
    contents: Vec<String>,
}

/// Wraps a backend and records every request it receives, for host-side
/// wire-fidelity assertions.
#[derive(Clone, Debug)]
struct Recording<M> {
    inner: M,
    seen: Arc<Mutex<Vec<Seen>>>,
}

impl<M: WasiModelCtx> Recording<M> {
    fn new(inner: M) -> (Self, Arc<Mutex<Vec<Seen>>>) {
        let seen = Arc::new(Mutex::new(Vec::new()));
        (
            Self {
                inner,
                seen: Arc::clone(&seen),
            },
            seen,
        )
    }
}

impl<M: WasiModelCtx + Clone> WasiModelCtx for Recording<M> {
    fn complete(&self, request: Request, tool_host: Arc<dyn ToolHost>) -> FutureResult<Answer> {
        self.seen.lock().expect("requests lock").push(Seen {
            system: request.system.clone(),
            contents: request.messages.iter().map(|message| message.content.clone()).collect(),
        });
        self.inner.complete(request, tool_host)
    }

    fn limits(&self) -> Limits {
        self.inner.limits()
    }
}

const fn answer(value: Value) -> Answer {
    Answer {
        value,
        usage: None,
        transcript: None,
    }
}

// ------------------------------------------------------------------------
// Scenarios (one per guest program; guest-side assertions live in
// `crates/test-programs/programs/`)
// ------------------------------------------------------------------------

#[tokio::test]
async fn model_echo_text() {
    let (recording, seen) = Recording::new(ModelDefault);
    let status =
        run_guest(test_utils::MODEL_ECHO_TEXT, vec![], recording).await.expect("guest runs");
    assert_eq!(status, ExitStatus::SUCCESS);

    // Wire fidelity: the request arrived at the backend intact.
    let requests = seen.lock().expect("requests lock");
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].system.as_deref(), Some("be terse"));
    assert_eq!(requests[0].contents, ["hi", "second"]);
}

#[tokio::test]
async fn model_echo_json() {
    let status =
        run_guest(test_utils::MODEL_ECHO_JSON, vec![], ModelDefault).await.expect("guest runs");
    assert_eq!(status, ExitStatus::SUCCESS);
}

#[tokio::test]
async fn model_echo_schema_rejected() {
    let status = run_guest(test_utils::MODEL_ECHO_SCHEMA_REJECTED, vec![], ModelDefault)
        .await
        .expect("guest runs");
    assert_eq!(status, ExitStatus::SUCCESS);
}

#[tokio::test]
async fn model_invalid_request() {
    let status =
        run_guest(test_utils::MODEL_INVALID_REQUEST, vec![], Unreached).await.expect("guest runs");
    assert_eq!(status, ExitStatus::SUCCESS);
}

#[tokio::test]
async fn model_schema_answer() {
    let status =
        run_guest(test_utils::MODEL_SCHEMA_ANSWER, vec![], Canned(json!({ "verdict": "pass" })))
            .await
            .expect("guest runs");
    assert_eq!(status, ExitStatus::SUCCESS);
}

#[tokio::test]
async fn model_answer_rejected() {
    let status =
        run_guest(test_utils::MODEL_ANSWER_REJECTED, vec![], Canned(json!({ "other": 1 })))
            .await
            .expect("guest runs");
    assert_eq!(status, ExitStatus::SUCCESS);
}

#[tokio::test]
async fn model_format_gate() {
    fn misshapen(marker: &str) -> Value {
        match marker {
            "object-for-text" => json!({ "a": 1 }),
            "string-for-json" => json!("nope"),
            "root-mismatch" => json!([]),
            "nested-mismatch" => json!({ "ui-surface": [] }),
            other => panic!("unexpected marker `{other}`"),
        }
    }
    let status = run_guest(test_utils::MODEL_FORMAT_GATE, vec![], Scripted(misshapen))
        .await
        .expect("guest runs");
    assert_eq!(status, ExitStatus::SUCCESS);
}

#[tokio::test]
async fn model_sections() {
    let (recording, seen) = Recording::new(ModelDefault);
    let status =
        run_guest(test_utils::MODEL_SECTIONS, vec![], recording).await.expect("guest runs");
    assert_eq!(status, ExitStatus::SUCCESS);

    // The assembled system channel crossed the boundary intact.
    let requests = seen.lock().expect("requests lock");
    assert_eq!(requests.len(), 1);
    assert_eq!(
        requests[0].system.as_deref(),
        Some("prefer {language}\n\na Rust reviewer\n\n- be Rust-idiomatic")
    );
}

#[tokio::test]
async fn model_tool_roundtrip() {
    let driver = ToolDriver {
        tool: "lookup",
        calls: 1,
        limits: Limits::default(),
    };
    let status =
        run_guest(test_utils::MODEL_TOOL_ROUNDTRIP, vec![], driver).await.expect("guest runs");
    assert_eq!(status, ExitStatus::SUCCESS);
}

#[tokio::test]
async fn model_tool_failure() {
    let driver = ToolDriver {
        tool: "lookup",
        calls: 1,
        limits: Limits::default(),
    };
    let status =
        run_guest(test_utils::MODEL_TOOL_FAILURE, vec![], driver).await.expect("guest runs");
    assert_eq!(status, ExitStatus::SUCCESS);
}

#[tokio::test]
async fn model_undeclared_tool() {
    let driver = ToolDriver {
        tool: "lookup",
        calls: 1,
        limits: Limits::default(),
    };
    let status =
        run_guest(test_utils::MODEL_UNDECLARED_TOOL, vec![], driver).await.expect("guest runs");
    assert_eq!(status, ExitStatus::SUCCESS);
}

#[tokio::test]
async fn model_tool_budget() {
    let driver = ToolDriver {
        tool: "lookup",
        calls: 2,
        limits: Limits {
            max_tool_calls: 1,
            ..Limits::default()
        },
    };
    let status =
        run_guest(test_utils::MODEL_TOOL_BUDGET, vec![], driver).await.expect("guest runs");
    assert_eq!(status, ExitStatus::SUCCESS);
}

#[tokio::test]
async fn model_tool_timeout() {
    let driver = ToolDriver {
        tool: "lookup",
        calls: 1,
        limits: Limits {
            tool_timeout: Duration::from_millis(50),
            ..Limits::default()
        },
    };
    let status =
        run_guest(test_utils::MODEL_TOOL_TIMEOUT, vec![], driver).await.expect("guest runs");
    assert_eq!(status, ExitStatus::SUCCESS);
}

#[tokio::test]
async fn model_out_of_order_results() {
    let status = run_guest(test_utils::MODEL_OUT_OF_ORDER_RESULTS, vec![], ParallelLookups)
        .await
        .expect("guest runs");
    assert_eq!(status, ExitStatus::SUCCESS);
}

#[tokio::test]
async fn model_workspace_tools() {
    let dir = scratch("tools");
    fs::write(dir.join("seed.txt"), "hello").expect("seeding workspace");

    let status =
        run_guest(test_utils::MODEL_WORKSPACE_TOOLS, vec![mount(&dir, true)], WorkspaceDriver)
            .await
            .expect("guest runs");
    assert_eq!(status, ExitStatus::SUCCESS);

    // The backend's write landed on the real filesystem.
    let written = fs::read_to_string(dir.join("out.txt")).expect("backend wrote out.txt");
    assert_eq!(written, "written");
    let _ = fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn model_workspace_denied() {
    // No mount and no grant: the host-injected tools must refuse to run.
    let status = run_guest(test_utils::MODEL_WORKSPACE_DENIED, vec![], WorkspaceDriver)
        .await
        .expect("guest runs");
    assert_eq!(status, ExitStatus::SUCCESS);
}

#[tokio::test]
async fn model_workspace_escape() {
    let dir = scratch("escape");
    let status = run_guest(test_utils::MODEL_WORKSPACE_ESCAPE, vec![mount(&dir, false)], Unreached)
        .await
        .expect("guest runs");
    assert_eq!(status, ExitStatus::SUCCESS);
    let _ = fs::remove_dir_all(&dir);
}
