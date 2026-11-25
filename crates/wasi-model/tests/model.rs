//! End-to-end tests for `omnia:model`: every scenario runs a real guest
//! component from `crates/test-programs` through the omnia runtime against
//! an inline scenario backend. The guest asserts what it observes across the
//! boundary (and traps on failure); the host side asserts wire fidelity and
//! filesystem effects.

#![cfg(not(target_arch = "wasm32"))]

use std::fs;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures::FutureExt as _;
use omnia::{Connect as _, ExitStatus, GuestEntry, Mount};
use omnia_test::host::{Backends, Deployment, ScriptedModel, scratch};
use omnia_test::{Exchange, SeenFormat, SeenWorker};
use omnia_wasi_model::{
    Answer, Budget, ConnectOptions, Error, Exhausted, Format, FutureResult, Grants, Limits,
    Message, ModelClient, ModelDefault, Request, Role, Schema, ToolHost, Usage, WasiModel,
};

// Every guest program in `crates/test-programs` must have a matching test
// here; a new program without one fails to compile.
test_programs::foreach_model!();

// ------------------------------------------------------------------------
// Harness
// ------------------------------------------------------------------------

async fn run_guest<M: ModelClient + Clone>(wasm: &str, mounts: Vec<Mount>, model: M) {
    let backends = Backends::defaults().await.model(model);
    let status = Deployment::new()
        .guest("guest", wasm)
        .mounts(mounts)
        .run_host::<WasiModel, _>(backends)
        .await
        .expect("guest runs");
    assert_eq!(status, ExitStatus::SUCCESS, "guest `{wasm}` failed");
}

// also requires the script to be exactly consumed; returns the model for its recordings
async fn run_scripted(wasm: &str, mounts: Vec<Mount>, model: ScriptedModel) -> ScriptedModel {
    run_guest(wasm, mounts, model.clone()).await;
    model.assert_exhausted();
    model
}

async fn echo(names: &[&str]) -> ModelDefault {
    let options = ConnectOptions {
        models: names.iter().map(|name| (*name).to_owned()).collect(),
    };
    ModelDefault::connect_with(options).await.expect("echo model")
}

// ------------------------------------------------------------------------
// Scenario backends
// ------------------------------------------------------------------------
//
// Most scenarios script `omnia_test::host::ScriptedModel`; the ones below
// exercise behaviour a FIFO script cannot express and stay by hand.

// Two concurrent `lookup` calls, answered in issue order, prove results
// correlate by id however the guest answers. By hand: a script is serial.
#[derive(Clone, Copy, Debug)]
struct ParallelLookups;

impl ModelClient for ParallelLookups {
    fn complete(&self, _request: Request, tool_host: Arc<dyn ToolHost>) -> FutureResult<Answer> {
        async move {
            let (first, second) = futures::join!(
                tool_host.call_tool("lookup".to_owned(), "1".to_owned()),
                tool_host.call_tool("lookup".to_owned(), "2".to_owned()),
            );
            let first = first?.map_err(|failure| anyhow::anyhow!("first call: {failure}"))?;
            let second = second?.map_err(|failure| anyhow::anyhow!("second call: {failure}"))?;
            Ok(format!("{first}|{second}").into())
        }
        .boxed()
    }

    fn limits(&self) -> Limits {
        Limits::default()
    }
}

// Calls an undeclared tool, ignores the hard failure and still answers: host
// enforcement must win over that `Ok`. By hand: a script propagates failure.
#[derive(Clone, Copy, Debug)]
struct IgnoringToolFailure;

impl ModelClient for IgnoringToolFailure {
    fn complete(&self, _request: Request, tool_host: Arc<dyn ToolHost>) -> FutureResult<Answer> {
        async move {
            assert!(
                tool_host.call_tool("lookup".to_owned(), "{}".to_owned()).await.is_err(),
                "undeclared tool is a hard failure"
            );
            Ok("should not reach the guest".into())
        }
        .boxed()
    }

    fn limits(&self) -> Limits {
        Limits::default()
    }
}

// fails a completion with the case its model is named for, delegating
// `default` to a script, and records each model so the run's exchanges are
// pinned
#[derive(Clone, Debug)]
struct Faulting {
    inner: ScriptedModel,
    opened: Arc<Mutex<Vec<String>>>,
}

impl ModelClient for Faulting {
    fn complete(&self, request: Request, tool_host: Arc<dyn ToolHost>) -> FutureResult<Answer> {
        self.opened.lock().expect("opened models").push(request.model.clone());
        let fault: anyhow::Error = match request.model.as_str() {
            "default" => return self.inner.complete(request, tool_host),
            "not-found" => Error::NotFound("not found by the suite".to_owned()).into(),
            "denied" => Error::Denied("denied by the suite".to_owned()).into(),
            "invalid" => Error::Invalid("invalid in the suite".to_owned()).into(),
            "unsupported" => Error::Unsupported("unsupported by the suite".to_owned()).into(),
            "unavailable" => Error::Unavailable("unavailable in the suite".to_owned()).into(),
            "exhausted" => Exhausted::new(Budget::Tokens, "exhausted in the suite").into(),
            _ => anyhow::anyhow!("an untyped backend failure"),
        };
        async move { Err(fault) }.boxed()
    }

    fn limits(&self) -> Limits {
        self.inner.limits()
    }
}

// answers the script with `check` stripped, so no candidate reaches the
// guest's check. By hand: a script honours the flag.
#[derive(Clone, Debug)]
struct Unchecked(ScriptedModel);

impl ModelClient for Unchecked {
    fn complete(&self, request: Request, tool_host: Arc<dyn ToolHost>) -> FutureResult<Answer> {
        self.0.complete(
            Request {
                check: false,
                ..request
            },
            tool_host,
        )
    }

    fn limits(&self) -> Limits {
        self.0.limits()
    }
}

// offers one candidate the check rejects, then answers with it anyway. By
// hand: a script stops on a rejection only when it runs out.
#[derive(Clone, Copy, Debug)]
struct Heedless;

impl ModelClient for Heedless {
    fn complete(&self, _request: Request, tool_host: Arc<dyn ToolHost>) -> FutureResult<Answer> {
        async move {
            let outcome = tool_host.check(FAIL.to_owned()).await?;
            assert!(outcome.is_err(), "the check rejects the candidate");
            Ok(FAIL.into())
        }
        .boxed()
    }

    fn limits(&self) -> Limits {
        Limits::default()
    }
}

// the candidates the `check_*` scenarios' scripted backend proposes
const PASS: &str = r#"{"verdict":"pass","findings":[]}"#;
const FAIL: &str = r#"{"verdict":"fail","findings":["x"]}"#;

fn check(candidate: &str, outcome: Result<&str, &str>) -> Exchange {
    Exchange {
        tool: "check".into(),
        arguments: candidate.into(),
        outcome: outcome.map(str::to_owned).map_err(str::to_owned),
    }
}

fn lookup(outcome: Result<&str, &str>) -> Exchange {
    Exchange {
        tool: "lookup".into(),
        arguments: "{}".into(),
        outcome: outcome.map(str::to_owned).map_err(str::to_owned),
    }
}

// one scripted call answered by the guest, then `answer`
fn lookup_turn(answer: &str) -> ScriptedModel {
    ScriptedModel::answering([answer]).calling(0, [("lookup", "{}")])
}

// read the seed, write `out.txt`, list the root, answer
fn workspace_turn() -> ScriptedModel {
    ScriptedModel::answering(["hello:out.txt,seed.txt"])
        .reading(0, "seed.txt")
        .writing(0, "out.txt", "written")
        .listing(0, "")
}

// what a fully served `workspace_turn` records
fn workspace_exchanges() -> Vec<Exchange> {
    vec![
        workspace_exchange("read", "seed.txt", "hello"),
        workspace_exchange("write", "out.txt", ""),
        workspace_exchange("list", "", "out.txt,seed.txt"),
    ]
}

fn workspace_exchange(tool: &str, path: &str, outcome: &str) -> Exchange {
    Exchange {
        tool: tool.into(),
        arguments: path.into(),
        outcome: Ok(outcome.into()),
    }
}

// ------------------------------------------------------------------------
// Scenarios (one per guest program; guest-side assertions live in
// `crates/test-programs/programs/model/`)
// ------------------------------------------------------------------------

#[tokio::test]
async fn model_echo_text() {
    let model =
        run_scripted(test_programs::MODEL_ECHO_TEXT, vec![], ScriptedModel::answering(["second"]))
            .await;

    // Wire fidelity: the request arrived at the backend intact.
    let seen = model.seen();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].system.as_deref(), Some("be terse"));
    assert_eq!(seen[0].messages, ["hi", "second"]);
    assert_eq!(seen[0].tools, Vec::<String>::new());
    assert!(seen[0].temperature.is_none());
    assert_eq!(seen[0].format, SeenFormat::Text);
    assert!(seen[0].workspace.is_none());
}

#[tokio::test]
async fn model_request_shape() {
    let workspace = scratch();
    let model = run_scripted(
        test_programs::MODEL_REQUEST_SHAPE,
        vec![workspace.mount(true)],
        ScriptedModel::answering(["hi"]),
    )
    .await;

    let seen = model.seen();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].tools, ["docs"]);
    assert_eq!(seen[0].temperature, Some(0.25));
    assert_eq!(seen[0].effort.as_deref(), Some("max"));
    assert_eq!(
        seen[0].worker,
        Some(SeenWorker {
            egress: vec!["registry.npmjs.org".to_owned(), "*.cursor.sh".to_owned()],
            read: vec!["~/.nvm/versions/node".to_owned()],
            env: vec![("VOLTA_HOME".to_owned(), "~/.volta".to_owned())],
        })
    );
}

#[tokio::test]
async fn model_worker_hosts() {
    let workspace = scratch();
    let model = ScriptedModel::answering(["hi"]);
    let grants = omnia::Grants::all().hosts(["*.npmjs.org", "api.cursor.sh"]);
    let guest =
        GuestEntry::new("guest", test_programs::MODEL_WORKER_HOSTS).command().grants(grants);

    let status = Deployment::new()
        .entry(guest)
        .mount(workspace.mount(true))
        .run_host::<WasiModel, _>(Backends::defaults().await.model(model.clone()))
        .await
        .expect("guest runs");
    assert_eq!(status, ExitStatus::SUCCESS, "the guest's assertions held");

    // the refused grants never reached the backend
    model.assert_exhausted();
    let seen = model.seen();
    assert_eq!(seen.len(), 1);
    assert_eq!(
        seen[0].worker,
        Some(SeenWorker {
            egress: ["registry.npmjs.org", "*.cdn.npmjs.org", "api.cursor.sh"]
                .map(String::from)
                .into(),
            read: vec![],
            env: vec![],
        })
    );
}

#[tokio::test]
async fn model_echo_default() {
    run_guest(test_programs::MODEL_ECHO_DEFAULT, vec![], echo(&["default", "fast"]).await).await;
}

#[tokio::test]
async fn model_error_codes() {
    let faulting = Faulting {
        inner: lookup_turn("unreached"),
        opened: Arc::default(),
    };
    run_guest(test_programs::MODEL_ERROR_CODES, vec![], faulting.clone()).await;

    // the host refused every other call before it reached the backend
    let opened = faulting.opened.lock().expect("opened models").clone();
    assert_eq!(
        opened,
        [
            "not-found",
            "denied",
            "invalid",
            "unsupported",
            "unavailable",
            "other",
            "exhausted",
            "default"
        ]
    );
    faulting.inner.assert_exhausted();
    assert!(faulting.inner.exchanges().is_empty(), "the wrong-kind result fails hard");
}

#[tokio::test]
async fn model_refused_request() {
    // Every request is refused before the backend runs; an empty script
    // proves it is never reached. The mount is the lend a worker grant's
    // shape rules are checked beneath, and holds the nested directory no
    // lend may take as its root.
    let workspace = scratch();
    fs::create_dir(workspace.path().join("nested")).expect("creating nested dir");
    run_scripted(
        test_programs::MODEL_REFUSED_REQUEST,
        vec![workspace.mount(true)],
        ScriptedModel::default(),
    )
    .await;
}

#[tokio::test]
async fn model_answer_flow() {
    let user_turn = "review the Rust code\n\nthe Rust crate\n\nInput: in\nOutput: out";
    let usage = Answer {
        answer: "hi".to_owned(),
        usage: Some(Usage {
            input_tokens: 3,
            output_tokens: 5,
            reasoning_tokens: Some(1),
            cache_read_tokens: Some(2),
            cache_write_tokens: None,
        }),
        transcript: None,
    };
    let script = [
        Answer::from(r#"{"verdict":"pass"}"#.to_owned()),
        usage,
        Answer::from(user_turn.to_owned()),
    ];
    let model =
        run_scripted(test_programs::MODEL_ANSWER_FLOW, vec![], ScriptedModel::replying(script))
            .await;
    let seen = model.seen();
    assert!(!seen[0].check, "no check was asked for");
    assert_eq!(model.exchanges(), vec![]);

    // the assembled system channel crossed the boundary intact
    assert_eq!(
        seen[2].system.as_deref(),
        Some("prefer {language}\n\na Rust reviewer\n\n- be Rust-idiomatic")
    );
}

#[tokio::test]
async fn model_check_flow() {
    let accepted = [PASS];
    let corrected = [FAIL, PASS];
    let plain = ["nope", "hi"];
    let recovered = [FAIL, PASS, r#"{"other":1}"#, PASS];
    let tools = [PASS, PASS];
    let exhausted = [FAIL, FAIL];
    let script = [&accepted[..], &corrected, &plain, &recovered, &tools, &exhausted].concat();

    // the `tools` turns each call `lookup` before answering
    let model = ScriptedModel::answering(script)
        .calling(9, [("lookup", r#"{"id":1}"#)])
        .calling(10, [("lookup", r#"{"id":1}"#)]);
    let model = run_scripted(test_programs::MODEL_CHECK_FLOW, vec![], model).await;

    // wire fidelity: `check` and the type's schema crossed the boundary
    let seen = model.seen();
    assert!(seen[0].check);
    assert_eq!(seen[0].system.as_deref(), Some("judge"));
    assert_eq!(seen[0].messages, ["judge this"]);
    let SeenFormat::Schema { name, schema } = &seen[0].format else {
        panic!("a question steers by schema: {:?}", seen[0].format);
    };
    assert_eq!(name, "verdict");
    assert!(schema.contains(r#""verdict""#) && schema.contains(r#""findings""#), "{schema}");
    let exchanges = model.exchanges();
    assert_eq!(exchanges.len(), 15);
    assert_eq!(exchanges[0], check(PASS, Ok("")));

    // the check's correction went back over the session verbatim
    assert_eq!(exchanges[1].tool, "check");
    assert_eq!(exchanges[1].arguments, FAIL);
    let correction = exchanges[1].outcome.clone().expect_err("the first candidate is rejected");
    assert!(correction.contains("## Previous answer (rejected)"), "{correction}");
    assert!(correction.contains(FAIL), "{correction}");
    assert!(correction.contains("## Findings\n\nverdict must be `pass`"), "{correction}");
    assert_eq!(exchanges[2], check(PASS, Ok("")));

    // a hand-written check
    assert_eq!(exchanges[3..5], [check("nope", Err("say `hi`, not `nope`")), check("hi", Ok(""))]);

    // a custom correction, then a mismatch corrected like any rejection
    let custom = format!("REJECTED {FAIL}: verdict must be `pass`; findings must be empty");
    assert_eq!(exchanges[5], check(FAIL, Err(&custom)));
    assert_eq!(exchanges[6], check(PASS, Ok("")));
    let correction = exchanges[7].outcome.clone().expect_err("the mismatch is rejected");
    assert!(correction.contains("does not match the expected shape"), "{correction}");
    assert_eq!(exchanges[8], check(PASS, Ok("")));

    // the guest's tools answer a call, and a question without them refuses it
    assert_eq!(exchanges[9].outcome, Ok("looked up 1".to_owned()));
    assert_eq!(exchanges[10], check(PASS, Ok("")));
    let refusal = exchanges[11].outcome.clone().expect_err("the guest passed no tools");
    assert!(refusal.contains("tool `lookup` has no handler"), "{refusal}");
    assert_eq!(exchanges[12], check(PASS, Ok("")));

    // every candidate the budget held was offered
    assert!(exchanges[13..].iter().all(|exchange| exchange.outcome.is_err()));
}

#[tokio::test]
async fn model_check_mismatch() {
    // Valid JSON of the wrong shape: the guest's `T` refuses it every time.
    let model = ScriptedModel::answering([r#"{"other":1}"#]);
    let model = run_scripted(test_programs::MODEL_CHECK_MISMATCH, vec![], model).await;
    let correction = model.exchanges()[0].outcome.clone().expect_err("rejected");
    assert!(correction.contains("does not match the expected shape"), "{correction}");
}

#[tokio::test]
async fn model_check_unheeded() {
    let unchecked = Unchecked(ScriptedModel::answering([PASS]));
    unheeded(unchecked, "the backend finished without running the check").await;
    unheeded(Heedless, "the backend finished on a candidate the check rejected").await;
}

// the guest fails unless its question ends in the backend error `expected`
async fn unheeded<M: ModelClient + Clone>(model: M, expected: &str) {
    let backends = Backends::defaults().await.model(model);
    let status = Deployment::new()
        .guest("guest", test_programs::MODEL_CHECK_UNHEEDED)
        .args([expected])
        .run_host::<WasiModel, _>(backends)
        .await
        .expect("guest runs");
    assert_eq!(status, ExitStatus::SUCCESS, "the guest saw another outcome than `{expected}`");
}

#[tokio::test]
async fn model_tool_flow() {
    // one turn per completion, each calling `lookup` before it answers
    let answers = ["42", "tool failed: no data", "42", "42", "42"];
    let model = (0..answers.len()).fold(ScriptedModel::answering(answers), |model, turn| {
        model.calling(turn, [("lookup", "{}")])
    });
    let model = run_scripted(test_programs::MODEL_TOOL_FLOW, vec![], model).await;
    assert_eq!(model.seen()[0].tools, ["lookup"]);

    // the closed results stream and the dropped calls stream each fail hard
    assert_eq!(model.exchanges(), [lookup(Ok("42")), lookup(Err("no data")), lookup(Ok("42"))]);
}

#[tokio::test]
async fn model_undeclared_tool() {
    run_guest(test_programs::MODEL_UNDECLARED_TOOL, vec![], IgnoringToolFailure).await;
}

#[tokio::test]
async fn model_tool_budget() {
    let model = ScriptedModel::answering(["42"])
        .calling(0, [("lookup", "{}"), ("lookup", "{}")])
        .limited(Limits {
            max_tool_calls: 1,
            ..Limits::default()
        });
    let model = run_scripted(test_programs::MODEL_TOOL_BUDGET, vec![], model).await;
    assert_eq!(model.exchanges(), [lookup(Ok("42"))], "the second call never reaches the guest");
}

#[tokio::test]
async fn model_tool_timeout() {
    let model = lookup_turn("42").limited(Limits {
        tool_timeout: Duration::from_millis(50),
        ..Limits::default()
    });
    let model = run_scripted(test_programs::MODEL_TOOL_TIMEOUT, vec![], model).await;
    assert!(model.exchanges().is_empty(), "the unanswered call fails hard");
}

#[tokio::test]
async fn model_oversize() {
    // One completion answered with an oversize tool output, one with an
    // oversize failure, then a correction: the guest's correction is
    // model-bound text, so the result cap applies to it as to a tool result.
    let model = ScriptedModel::answering(["42", "42", FAIL])
        .calling(0, [("lookup", "{}")])
        .calling(1, [("lookup", "{}")])
        .limited(Limits {
            max_result_bytes: 4,
            ..Limits::default()
        });
    let model = run_scripted(test_programs::MODEL_OVERSIZE, vec![], model).await;
    assert!(model.exchanges().is_empty(), "every oversize text fails hard");
}

#[tokio::test]
async fn model_out_of_order_results() {
    run_guest(test_programs::MODEL_OUT_OF_ORDER_RESULTS, vec![], ParallelLookups).await;
}

#[tokio::test]
async fn model_workspace_tools() {
    let workspace = scratch();
    fs::write(workspace.path().join("seed.txt"), "hello").expect("seeding workspace");
    fs::create_dir(workspace.path().join("docs")).expect("creating docs dir");
    fs::write(workspace.path().join("docs").join("a.txt"), "a").expect("seeding docs");

    let model = ScriptedModel::answering(["hello:docs,out.txt,seed.txt"])
        .reading(0, "seed.txt")
        .writing(0, "out.txt", "written")
        .listing(0, "")
        .listing(0, "docs");
    let model =
        run_scripted(test_programs::MODEL_WORKSPACE_TOOLS, vec![workspace.mount(true)], model)
            .await;

    // The host-injected tools read the seed and listed the root and a
    // subdirectory; the write landed on the real filesystem, and
    // `local_path` resolved to this mount.
    assert_eq!(
        model.exchanges(),
        [
            workspace_exchange("read", "seed.txt", "hello"),
            workspace_exchange("write", "out.txt", ""),
            workspace_exchange("list", "", "docs,out.txt,seed.txt"),
            workspace_exchange("list", "docs", "a.txt"),
        ]
    );
    assert_eq!(fs::read_to_string(workspace.path().join("out.txt")).expect("out.txt"), "written");
    assert_eq!(model.lent(), [Some(workspace.path().to_path_buf())]);
}

#[tokio::test]
async fn model_workspace_limits() {
    // the host's private caps: 4 MiB read and write, 4096 listed entries
    const MAX_BYTES: usize = 4 * 1024 * 1024;
    const MAX_LIST_ENTRIES: usize = 4096;

    let workspace = scratch();
    fs::write(workspace.path().join("big.bin"), vec![0_u8; MAX_BYTES + 1])
        .expect("seeding big.bin");
    for i in 0..MAX_LIST_ENTRIES {
        fs::write(workspace.path().join(format!("{i}.txt")), []).expect("seeding entries");
    }

    let model = ScriptedModel::answering(["read", "write", "list"])
        .reading(0, "big.bin")
        .writing(1, "out.bin", vec![0_u8; MAX_BYTES + 1])
        .listing(2, "");
    let model =
        run_scripted(test_programs::MODEL_WORKSPACE_LIMITS, vec![workspace.mount(true)], model)
            .await;

    assert!(model.exchanges().is_empty(), "every capped step fails hard");
    assert!(!workspace.path().join("out.bin").exists(), "the oversize write never lands");
}

#[tokio::test]
async fn model_workspace_denied() {
    // No mount and no grant: the host-injected tools must refuse to run.
    let model = run_scripted(test_programs::MODEL_WORKSPACE_DENIED, vec![], workspace_turn()).await;
    assert!(model.exchanges().is_empty(), "the first read fails hard");
    assert_eq!(model.lent(), [None]);
}

#[tokio::test]
async fn model_workspace_escape() {
    let workspace = scratch();
    run_scripted(
        test_programs::MODEL_WORKSPACE_ESCAPE,
        vec![workspace.mount(false)],
        ScriptedModel::default(),
    )
    .await;
}

#[tokio::test]
async fn model_workspace_subpath() {
    let workspace = scratch();
    let nested = workspace.path().join("nested");
    fs::create_dir(&nested).expect("creating nested dir");
    fs::write(nested.join("seed.txt"), "hello").expect("seeding nested workspace");

    let model = run_scripted(
        test_programs::MODEL_WORKSPACE_SUBPATH,
        vec![workspace.mount(true)],
        workspace_turn(),
    )
    .await;

    assert_eq!(model.exchanges(), workspace_exchanges());
    assert_eq!(fs::read_to_string(nested.join("out.txt")).expect("nested/out.txt"), "written");
    assert!(!workspace.path().join("out.txt").exists(), "write stays under the subpath");
    assert_eq!(model.lent(), [Some(nested)]);
}

#[tokio::test]
async fn model_workspace_readonly() {
    let workspace = scratch();
    fs::write(workspace.path().join("seed.txt"), "hello").expect("seeding workspace");
    let model = run_scripted(
        test_programs::MODEL_WORKSPACE_READONLY,
        vec![workspace.mount(false)],
        workspace_turn(),
    )
    .await;
    assert_eq!(model.exchanges(), [workspace_exchanges()[0].clone()], "the write fails hard");
}

// ------------------------------------------------------------------------
// Prompt formatting and candidate extraction
// ------------------------------------------------------------------------

const VERDICT_PASS: &str = r#"{"verdict":"pass"}"#;

fn verdict_schema() -> Format {
    Format::Schema(Schema {
        name: "verdict".to_owned(),
        schema: r#"{"type":"object"}"#.to_owned(),
    })
}

fn candidate(text: &str) -> String {
    verdict_schema().candidate(text)
}

#[test]
fn mixed_roles() {
    let messages = [(Role::User, "hi"), (Role::Assistant, "yo"), (Role::System, "note")];
    let prompt = Request {
        model: "default".to_owned(),
        system: Some("be terse".to_owned()),
        messages: messages
            .into_iter()
            .map(|(role, content)| Message {
                role,
                content: content.to_owned(),
            })
            .collect(),
        generation: None,
        format: Format::Text,
        tools: vec![],
        grants: Grants { workspace: None },
        check: false,
    }
    .to_string();
    assert!(
        prompt.starts_with("be terse\n\nhi\n\n[assistant]\nyo\n\n[system]\nnote\n\n"),
        "{prompt}"
    );
    assert!(prompt.contains("plain text"), "{prompt}");
}

#[test]
fn text_passthrough() {
    assert_eq!(Format::Text.candidate("  plain {not: json}  "), "  plain {not: json}  ");
}

#[test]
fn json_document() {
    assert_eq!(Format::Json.candidate(VERDICT_PASS), VERDICT_PASS);
}

#[test]
fn fenced_json() {
    assert_eq!(candidate("```json\n{\"verdict\":\"pass\"}\n```"), VERDICT_PASS);
}

// A scalar answer has no bracket to open it, so the fence is its only way in.
#[test]
fn fenced_scalar() {
    assert_eq!(candidate("```json\n\"pass\"\n```"), r#""pass""#);
}

#[test]
fn json_with_preamble() {
    assert_eq!(candidate("Done.\n{\"verdict\":\"pass\"}\n"), VERDICT_PASS);
}

// The full document beats a worked example, a repeated fragment, and a bracketed citation.
#[test]
fn largest_value() {
    assert_eq!(
        candidate(
            "For example {\"verdict\":\"fail\"} — and mine:\n{\"findings\":[],\"verdict\":\"pass\"}"
        ),
        r#"{"findings":[],"verdict":"pass"}"#
    );
    let document = r#"{"requirements":[{"scenarios":[],"subject":"a.run"}]}"#;
    assert_eq!(
        candidate(&format!("{document}\n{{\"scenarios\":[],\"subject\":\"a.run\"}}")),
        document
    );
    assert_eq!(candidate("{\"verdict\":\"pass\"}\nSee [1] and [2]."), VERDICT_PASS);
}

// Blocks that fail to parse whole are skipped without mining literals inside them,
// even when a code fence holds a larger JSON literal than the answer.
#[test]
fn non_json_blocks() {
    for text in [
        "[thinking] weighing the claims.\n{\"verdict\":\"pass\"}",
        "{thinking} done.\n```json\n{\"verdict\":\"pass\"}\n```",
        "Done [thinking] with {care}; see [[note]] and {{name}}.\n{\"verdict\":\"pass\"}",
        "Scored [1 point] as a [true story] — [3 items, mostly]:\n{\"verdict\":\"pass\"}",
        "```python\nxs = [x for x in xs]\n```\n{\"verdict\":\"pass\"}",
        "```js\nconst reply = { \"verdict\": verdict };\n```\n{\"verdict\":\"pass\"}",
        "```python\nconfig = {\"model\":\"gpt-4\",\"temperature\":0,\"stream\":false}\n```\n{\"verdict\":\"pass\"}",
        "{\"verdict\":\"pass\"}\nThe {\"verdict\"} key is required.",
        "The object opens with a { like so:\n```json\n{\"verdict\":\"pass\"}\n```",
    ] {
        assert_eq!(candidate(text), VERDICT_PASS, "{text}");
    }
}

// A bracket or an escaped quote inside a string does not end the block.
#[test]
fn brackets_in_strings() {
    let answer = r#"{"finding":"an unmatched } and \" quote","verdict":"pass"}"#;
    assert_eq!(candidate(&format!("Note:\n{answer}")), answer);
}

// A mid-line "```" is content rather than a fence delimiter, whether inside a
// JSON string or in prose.
#[test]
fn fence_delimiter_mid_line() {
    let answer = r#"{"finding":"wrap code in ``` fences","verdict":"pass"}"#;
    assert_eq!(candidate(&format!("Note:\n{answer}")), answer);
    assert_eq!(candidate(&format!("```json\n{answer}\n```")), answer);
    assert_eq!(
        candidate("Open a fence with ``` on its own line.\n{\"verdict\":\"pass\"}"),
        VERDICT_PASS
    );
}

// A truncated or invalid document comes back whole rather than yielding a
// well-formed inner fragment.
#[test]
fn malformed_document() {
    for text in [
        "{\n  \"requirements\": [{\"scenarios\":[],\"subject\":\"a.run\"}],\n",
        "```json\n{\n  \"requirements\": [{\"scenarios\":[],\"subject\":\"a.run\"}],\n```",
        "Here is my answer: {\"findings\": [], \"verdict\": \"pass\"",
        "{\"verdict\": \"pass\", // reason\n  \"findings\": [{\"claim\":\"a\"}]}",
    ] {
        assert_eq!(candidate(text), text);
    }
}

#[test]
fn no_json() {
    assert_eq!(candidate("not json"), "not json");
}
