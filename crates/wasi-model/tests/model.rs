//! End-to-end tests for `omnia:model/completion`: every scenario runs a real
//! guest component from `crates/test-programs` through the omnia runtime
//! against an inline scenario backend. The guest asserts what it observes
//! across the boundary (and traps on failure); the host side asserts wire
//! fidelity and filesystem effects.

#![cfg(not(target_arch = "wasm32"))]

use std::fs;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures::FutureExt as _;
use omnia::{ExitStatus, Mount, Provides};
use omnia_wasi_model::{
    Answer, FutureResult, Limits, ModelDefault, Request, ToolHost, Usage, WasiModel, WasiModelCtx,
};
use serde_json::{Value, json};

// Every guest program in `crates/test-programs` must have a matching test
// here; a new program without one fails to compile.
test_utils::foreach_model!();

// ------------------------------------------------------------------------
// Harness
// ------------------------------------------------------------------------

/// The store's backend bundle: just the model backend under test.
#[derive(Clone, Debug)]
struct Backends<M>(M);

impl<M: WasiModelCtx + Clone> Provides<WasiModel> for Backends<M> {
    fn borrow(&mut self) -> &mut dyn WasiModelCtx {
        &mut self.0
    }
}

/// Run one guest program against `model`, requiring a clean exit; scenarios
/// that expect failure call `test_utils::run_command` directly.
async fn run_guest<M: WasiModelCtx + Clone>(wasm: &str, mounts: Vec<Mount>, model: M) {
    let status = test_utils::run_host::<WasiModel, _>(wasm, mounts, Backends(model))
        .await
        .expect("guest runs");
    assert_eq!(status, ExitStatus::SUCCESS, "guest `{wasm}` failed");
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
        async move { Ok(value.into()) }.boxed()
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
        async move { Ok(value.into()) }.boxed()
    }
}

/// Calls the `lookup` tool `calls` times in sequence and answers with the
/// last outcome (a failure becomes model-visible repair text).
#[derive(Clone, Debug)]
struct ToolDriver {
    tool: &'static str,
    calls: u32,
    limits: Limits,
}

impl Default for ToolDriver {
    fn default() -> Self {
        Self {
            tool: "lookup",
            calls: 1,
            limits: Limits::default(),
        }
    }
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
            Ok(Value::String(last).into())
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
            Ok(Value::String(format!("{first}|{second}")).into())
        }
        .boxed()
    }
}

/// Drives the host-injected workspace tools: reads the seed file, writes the
/// lent `local_path`, and answers with the seed content plus the sorted listing.
#[derive(Clone, Copy, Debug)]
struct WorkspaceDriver;

impl WasiModelCtx for WorkspaceDriver {
    fn complete(&self, _request: Request, tool_host: Arc<dyn ToolHost>) -> FutureResult<Answer> {
        async move {
            let seed = tool_host.read("seed.txt".to_owned()).await?;
            let path = tool_host
                .local_path()
                .map(|path| path.to_string_lossy().into_owned())
                .unwrap_or_default();
            tool_host.write("out.txt".to_owned(), path.into_bytes()).await?;
            let mut names: Vec<String> =
                tool_host.list(String::new()).await?.into_iter().map(|entry| entry.name).collect();
            names.sort();
            let text = format!("{}:{}", String::from_utf8_lossy(&seed), names.join(","));
            Ok(Value::String(text).into())
        }
        .boxed()
    }
}

/// Answers with a fully specified [`Answer`], for usage projection.
#[derive(Clone, Debug)]
struct CannedAnswer(Answer);

impl WasiModelCtx for CannedAnswer {
    fn complete(&self, _request: Request, _tool_host: Arc<dyn ToolHost>) -> FutureResult<Answer> {
        let answer = self.0.clone();
        async move { Ok(answer) }.boxed()
    }
}

/// Calls an undeclared tool, ignores the hard failure, and still answers —
/// host enforcement must win over that `Ok`.
#[derive(Clone, Copy, Debug)]
struct IgnoringToolFailure;

impl WasiModelCtx for IgnoringToolFailure {
    fn complete(&self, _request: Request, tool_host: Arc<dyn ToolHost>) -> FutureResult<Answer> {
        async move {
            assert!(
                tool_host.call_tool("lookup".to_owned(), "{}".to_owned()).await.is_err(),
                "undeclared tool is a hard failure"
            );
            Ok(Value::String("should not reach the guest".to_owned()).into())
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
    mcp: Vec<String>,
    temperature: Option<f32>,
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
            mcp: request.mcp_servers().iter().map(|grant| grant.name.clone()).collect(),
            temperature: request.generation.as_ref().and_then(|generation| generation.temperature),
        });
        self.inner.complete(request, tool_host)
    }

    fn limits(&self) -> Limits {
        self.inner.limits()
    }
}

// ------------------------------------------------------------------------
// Scenarios (one per guest program; guest-side assertions live in
// `crates/test-programs/programs/model/`)
// ------------------------------------------------------------------------

#[tokio::test]
async fn model_echo_text() {
    let (recording, seen) = Recording::new(ModelDefault);
    run_guest(test_utils::MODEL_ECHO_TEXT, vec![], recording).await;

    // Wire fidelity: the request arrived at the backend intact.
    let requests = seen.lock().expect("requests lock");
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].system.as_deref(), Some("be terse"));
    assert_eq!(requests[0].contents, ["hi", "second"]);
    assert!(requests[0].mcp.is_empty());
    assert!(requests[0].temperature.is_none());
    drop(requests);
}

#[tokio::test]
async fn model_request_shape() {
    let (recording, seen) = Recording::new(ModelDefault);
    run_guest(test_utils::MODEL_REQUEST_SHAPE, vec![], recording).await;

    let requests = seen.lock().expect("requests lock");
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].mcp, ["docs"]);
    assert_eq!(requests[0].temperature, Some(0.25));
    drop(requests);
}

#[tokio::test]
async fn model_echo_json() {
    run_guest(test_utils::MODEL_ECHO_JSON, vec![], ModelDefault).await;
}

#[tokio::test]
async fn model_echo_schema_rejected() {
    run_guest(test_utils::MODEL_ECHO_SCHEMA_REJECTED, vec![], ModelDefault).await;
}

#[tokio::test]
async fn model_invalid_request() {
    run_guest(test_utils::MODEL_INVALID_REQUEST, vec![], Unreached).await;
}

#[tokio::test]
async fn model_schema_answer() {
    run_guest(test_utils::MODEL_SCHEMA_ANSWER, vec![], Canned(json!({ "verdict": "pass" }))).await;
}

#[tokio::test]
async fn model_usage() {
    let answer = Answer {
        value: json!("hi"),
        usage: Some(Usage {
            input_tokens: 3,
            output_tokens: 5,
            reasoning_tokens: Some(1),
        }),
        transcript: None,
    };
    run_guest(test_utils::MODEL_USAGE, vec![], CannedAnswer(answer)).await;
}

#[tokio::test]
async fn model_answer_rejected() {
    run_guest(test_utils::MODEL_ANSWER_REJECTED, vec![], Canned(json!({ "other": 1 }))).await;
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
    run_guest(test_utils::MODEL_FORMAT_GATE, vec![], Scripted(misshapen)).await;
}

#[tokio::test]
async fn model_sections() {
    let (recording, seen) = Recording::new(ModelDefault);
    run_guest(test_utils::MODEL_SECTIONS, vec![], recording).await;

    // The assembled system channel crossed the boundary intact.
    let requests = seen.lock().expect("requests lock");
    assert_eq!(requests.len(), 1);
    assert_eq!(
        requests[0].system.as_deref(),
        Some("prefer {language}\n\na Rust reviewer\n\n- be Rust-idiomatic")
    );
    drop(requests);
}

#[tokio::test]
async fn model_tool_roundtrip() {
    run_guest(test_utils::MODEL_TOOL_ROUNDTRIP, vec![], ToolDriver::default()).await;
}

#[tokio::test]
async fn model_tool_failure() {
    run_guest(test_utils::MODEL_TOOL_FAILURE, vec![], ToolDriver::default()).await;
}

#[tokio::test]
async fn model_undeclared_tool() {
    run_guest(test_utils::MODEL_UNDECLARED_TOOL, vec![], IgnoringToolFailure).await;
}

#[tokio::test]
async fn model_tool_budget() {
    let driver = ToolDriver {
        calls: 2,
        limits: Limits {
            max_tool_calls: 1,
            ..Limits::default()
        },
        ..ToolDriver::default()
    };
    run_guest(test_utils::MODEL_TOOL_BUDGET, vec![], driver).await;
}

#[tokio::test]
async fn model_tool_timeout() {
    let driver = ToolDriver {
        limits: Limits {
            tool_timeout: Duration::from_millis(50),
            ..Limits::default()
        },
        ..ToolDriver::default()
    };
    run_guest(test_utils::MODEL_TOOL_TIMEOUT, vec![], driver).await;
}

#[tokio::test]
async fn model_tool_oversize() {
    let driver = ToolDriver {
        limits: Limits {
            max_result_bytes: 4,
            ..Limits::default()
        },
        ..ToolDriver::default()
    };
    run_guest(test_utils::MODEL_TOOL_OVERSIZE, vec![], driver).await;
}

#[tokio::test]
async fn model_results_closed() {
    run_guest(test_utils::MODEL_RESULTS_CLOSED, vec![], ToolDriver::default()).await;
}

#[tokio::test]
async fn model_stale_result() {
    run_guest(test_utils::MODEL_STALE_RESULT, vec![], ToolDriver::default()).await;
}

#[tokio::test]
async fn model_out_of_order_results() {
    run_guest(test_utils::MODEL_OUT_OF_ORDER_RESULTS, vec![], ParallelLookups).await;
}

#[tokio::test]
async fn model_workspace_tools() {
    let workspace = test_utils::scratch("model_tools");
    fs::write(workspace.path().join("seed.txt"), "hello").expect("seeding workspace");

    run_guest(test_utils::MODEL_WORKSPACE_TOOLS, vec![workspace.mount(true)], WorkspaceDriver)
        .await;

    // The backend's write landed on the real filesystem, and `local_path`
    // resolved to this mount.
    let written =
        fs::read_to_string(workspace.path().join("out.txt")).expect("backend wrote out.txt");
    assert_eq!(written, workspace.path().to_string_lossy());
}

#[tokio::test]
async fn model_workspace_denied() {
    // No mount and no grant: the host-injected tools must refuse to run.
    run_guest(test_utils::MODEL_WORKSPACE_DENIED, vec![], WorkspaceDriver).await;
}

#[tokio::test]
async fn model_workspace_escape() {
    let workspace = test_utils::scratch("model_escape");
    run_guest(test_utils::MODEL_WORKSPACE_ESCAPE, vec![workspace.mount(false)], Unreached).await;
}

#[tokio::test]
async fn model_workspace_subpath() {
    let workspace = test_utils::scratch("model_subpath");
    let nested = workspace.path().join("nested");
    fs::create_dir(&nested).expect("creating nested dir");
    fs::write(nested.join("seed.txt"), "hello").expect("seeding nested workspace");

    run_guest(test_utils::MODEL_WORKSPACE_SUBPATH, vec![workspace.mount(true)], WorkspaceDriver)
        .await;

    let written = fs::read_to_string(nested.join("out.txt")).expect("backend wrote nested/out.txt");
    assert_eq!(written, nested.to_string_lossy());
    assert!(!workspace.path().join("out.txt").exists(), "write stays under the subpath");
}

#[tokio::test]
async fn model_workspace_readonly() {
    let workspace = test_utils::scratch("model_readonly");
    fs::write(workspace.path().join("seed.txt"), "hello").expect("seeding workspace");
    run_guest(test_utils::MODEL_WORKSPACE_READONLY, vec![workspace.mount(false)], WorkspaceDriver)
        .await;
}

#[tokio::test]
async fn model_workspace_unauthorized() {
    let workspace = test_utils::scratch("model_unauthorized");
    fs::create_dir(workspace.path().join("nested")).expect("creating nested dir");
    run_guest(test_utils::MODEL_WORKSPACE_UNAUTHORIZED, vec![workspace.mount(true)], Unreached)
        .await;
}
