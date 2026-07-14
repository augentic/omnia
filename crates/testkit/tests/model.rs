//! Public model test-double behavior, on both faces: the guest-side `Model`
//! and the host-side `WasiModelCtx` served by the same `Scripted` queue.

use std::sync::Arc;

use futures::FutureExt as _;
use futures::executor::block_on;
use omnia_guest::model::{
    Effort, Error, Format, Function, Generation, McpGrant, Message, Model, Reply, Request, Role,
    SchemaFormat, Tool, Usage,
};
use omnia_testkit::model::{Harness, Scripted, mcp_grants};
use omnia_wasi_model::{
    Answer, DirEntry, FutureResult, Reference, ToolHost, VerifyReport, WasiModelCtx,
};
use serde_json::json;

#[test]
fn scripting_order() {
    let model = Scripted::answers(["first", "second"]);

    assert_eq!(complete(&model, request("one")).unwrap().answer, "first");
    assert_eq!(complete(&model, request("two")).unwrap().answer, "second");
    model.assert_exhausted();
}

#[test]
fn error_injection() {
    let expected = Error::ToolFailed("resolver unavailable".to_owned());
    let model = Scripted::new([
        Err(expected.clone()),
        Ok(Reply {
            answer: "recovered".to_owned(),
            usage: Some(Usage {
                input_tokens: 3,
                output_tokens: 1,
                reasoning_tokens: None,
            }),
        }),
    ]);

    assert_eq!(complete(&model, request("one")).unwrap_err(), expected);
    assert_eq!(complete(&model, request("two")).unwrap().answer, "recovered");
    model.assert_exhausted();
}

#[test]
fn exhaustion() {
    let model = Scripted::reply("only");

    assert_eq!(complete(&model, request("one")).unwrap().answer, "only");
    assert_eq!(
        complete(&model, request("two")).unwrap_err(),
        Error::Backend("model script exhausted".to_owned())
    );
    model.assert_exhausted();
}

#[test]
fn request_recording() {
    let scripted = Scripted::reply("ok");
    let harness = Harness::new(scripted);
    let request = complete_request();

    assert_eq!(complete(&harness, request.clone()).unwrap().answer, "ok");
    assert_eq!(harness.requests(), vec![request.clone()]);

    let requests = harness.requests();
    let grants = mcp_grants(&requests[0]);
    assert_eq!(
        grants,
        vec![match &request.tools[1] {
            Tool::Mcp(grant) => grant,
            Tool::Function(_) => unreachable!(),
        }]
    );
}

#[test]
fn snapshot_sharing() {
    let harness = Harness::new(Scripted::answers(["one", "two"]));
    let other = harness.clone();

    let thread = std::thread::spawn(move || complete(&other, request("thread")));
    let main_answer = complete(&harness, request("main")).unwrap().answer;
    let thread_answer = thread.join().unwrap().unwrap().answer;
    let mut answers = [main_answer, thread_answer];
    answers.sort();
    assert_eq!(answers, ["one", "two"]);

    let mut messages = harness
        .requests()
        .into_iter()
        .map(|request| request.messages[0].content.clone())
        .collect::<Vec<_>>();
    messages.sort();
    assert_eq!(messages, ["main", "thread"]);
}

#[test]
fn host_scripting_order() {
    let backend = Scripted::answers(["first", "second"]);

    assert_eq!(host_complete(&backend).unwrap().value, json!("first"));
    assert_eq!(host_complete(&backend).unwrap().value, json!("second"));
    backend.assert_exhausted();
}

#[test]
fn host_exhaustion() {
    let backend = Scripted::json(json!({ "verdict": "pass" }));

    assert_eq!(host_complete(&backend).unwrap().value, json!({ "verdict": "pass" }));
    let error = host_complete(&backend).unwrap_err();
    assert_eq!(error.to_string(), "model script exhausted");
}

#[test]
fn host_error_injection() {
    let backend = Scripted::new([Err(Error::ToolFailed("resolver unavailable".to_owned()))]);

    let error = host_complete(&backend).unwrap_err();
    assert_eq!(error.to_string(), "tool failed: resolver unavailable");
    backend.assert_exhausted();
}

// The same queue serves both faces: a JSON answer reaches the host face as
// the value and the guest face as its serialization.
#[test]
fn shared_queue_across_faces() {
    let value = json!({ "verdict": "pass" });
    let scripted = Scripted::json(value.clone());

    assert_eq!(host_complete(&scripted.clone()).unwrap().value, value);
    scripted.assert_exhausted();

    let scripted = Scripted::json(value.clone());
    let reply = complete(&scripted, request("review")).unwrap();
    assert_eq!(serde_json::from_str::<serde_json::Value>(&reply.answer).unwrap(), value);
    scripted.assert_exhausted();
}

fn complete(model: &impl Model, request: Request) -> Result<Reply, Error> {
    block_on(model.create(request))
}

fn host_complete(backend: &Scripted) -> anyhow::Result<Answer> {
    block_on(backend.complete(wire_request(), Arc::new(NoTools)))
}

fn wire_request() -> omnia_wasi_model::Request {
    omnia_wasi_model::Request {
        model: None,
        system: None,
        messages: vec![omnia_wasi_model::Message {
            role: omnia_wasi_model::Role::User,
            content: "question".to_owned(),
        }],
        generation: None,
        format: omnia_wasi_model::Format::Text,
        tools: vec![],
        grants: omnia_wasi_model::Grants {
            references: None,
            workspace: None,
            verify: vec![],
        },
    }
}

// The doubles never call tools; every method fails loud if one does.
struct NoTools;

impl ToolHost for NoTools {
    fn resolve(&self, _reference: Reference) -> FutureResult<Vec<u8>> {
        no_tools()
    }

    fn read(&self, _path: String) -> FutureResult<Vec<u8>> {
        no_tools()
    }

    fn list(&self, _path: String) -> FutureResult<Vec<DirEntry>> {
        no_tools()
    }

    fn write(&self, _path: String, _bytes: Vec<u8>) -> FutureResult<()> {
        no_tools()
    }

    fn verify(&self, _check: String) -> FutureResult<VerifyReport> {
        no_tools()
    }
}

fn no_tools<T>() -> FutureResult<T> {
    async { Err(anyhow::anyhow!("the scripted double never calls tools")) }.boxed()
}

fn request(content: &str) -> Request {
    Request {
        messages: vec![Message {
            role: Role::User,
            content: content.to_owned(),
        }],
        ..Request::default()
    }
}

fn complete_request() -> Request {
    Request {
        model: Some("test-model".to_owned()),
        system: Some("system".to_owned()),
        messages: vec![
            Message {
                role: Role::System,
                content: "context".to_owned(),
            },
            Message {
                role: Role::User,
                content: "question".to_owned(),
            },
            Message {
                role: Role::Assistant,
                content: "prior".to_owned(),
            },
        ],
        generation: Some(Generation {
            temperature: Some(0.2),
            top_p: Some(0.9),
            max_tokens: Some(64),
            stop: vec!["done".to_owned()],
            seed: Some(7),
            effort: Some(Effort::High),
        }),
        format: Format::Schema(SchemaFormat {
            name: "result".to_owned(),
            schema: r#"{"type":"object"}"#.to_owned(),
        }),
        tools: vec![
            Tool::Function(Function {
                name: "lookup".to_owned(),
                description: "Look up a value".to_owned(),
                parameters: r#"{"type":"object"}"#.to_owned(),
            }),
            Tool::Mcp(McpGrant {
                name: "docs".to_owned(),
                tools: vec!["search".to_owned()],
                url: "https://mcp.example.test".to_owned(),
            }),
        ],
        references: Some("reference-guest".to_owned()),
        verify: vec!["cargo-check".to_owned()],
        lend_workspace: true,
    }
}
