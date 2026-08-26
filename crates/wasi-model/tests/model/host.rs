use std::sync::Arc;

use futures::FutureExt as _;
use futures::executor::block_on;
use omnia_wasi_model::{
    Answer, Candidate, DirEntry, Error, Format, Function, FutureResult, Grants, Message,
    ModelDefault, Reply, Request, Role, Schema, Tool, ToolHost, ToolOutcome, WasiModelCtx,
};
use serde_json::{Value, json};

#[test]
fn json_document() {
    assert_eq!(
        Format::Json.parse(r#"{"verdict":"pass"}"#).unwrap(),
        Candidate::Valid(json!({ "verdict": "pass" }))
    );
    let err = Format::Json.parse("not json").unwrap_err();
    assert!(err.contains("does not contain JSON"), "unexpected: {err}");
}

#[test]
fn fenced_json() {
    let fenced = "```json\n{\"verdict\":\"pass\"}\n```";
    assert_eq!(
        verdict_schema().parse(fenced).unwrap(),
        Candidate::Valid(json!({ "verdict": "pass" }))
    );
}

#[test]
fn json_with_preamble() {
    let text = "Done.\n{\"verdict\":\"pass\"}\n";
    assert_eq!(Format::Json.parse(text).unwrap(), Candidate::Valid(json!({ "verdict": "pass" })));
}

#[test]
fn later_matching_candidate() {
    let text = "findings: []\n{\"outcome\":\"completed\",\"source\":\"model-assisted\"}";
    assert_eq!(
        report_schema().parse(text).unwrap(),
        Candidate::Valid(json!({ "outcome": "completed", "source": "model-assisted" }))
    );
}

#[test]
fn invalid_fence() {
    let text = "```json\n[]\n```\n{\"verdict\":\"pass\"}";
    assert_eq!(
        verdict_schema().parse(text).unwrap(),
        Candidate::Valid(json!({ "verdict": "pass" }))
    );
}

#[test]
fn invalid_candidate() {
    let Candidate::Invalid { value, reason } = report_schema().parse("[]").unwrap() else {
        panic!("expected an invalid candidate");
    };
    assert_eq!(value, json!([]));
    assert!(reason.contains("does not conform to schema `phase-report`"), "unexpected: {reason}");
    assert!(reason.contains("at root"), "unexpected: {reason}");
}

#[test]
fn no_matching_candidate() {
    let format = verdict_schema();
    let fenced = r#"{"note":"```json\n{\"verdict\":\"pass\"}\n```"}"#;
    let Candidate::Invalid { value, .. } = format.parse(fenced).unwrap() else {
        panic!("expected an invalid candidate");
    };
    assert_eq!(value, json!({ "note": "```json\n{\"verdict\":\"pass\"}\n```" }));

    let quoted = r#""use {\"verdict\":\"pass\"} as the answer""#;
    let Candidate::Invalid { value, .. } = format.parse(quoted).unwrap() else {
        panic!("expected an invalid candidate");
    };
    assert_eq!(value, json!("use {\"verdict\":\"pass\"} as the answer"));
}

#[test]
fn reject_object() {
    Format::Text.check(&json!("hi")).unwrap();
    let err = Format::Text.check(&json!({ "a": 1 })).unwrap_err();
    assert!(err.contains("not a JSON string"), "unexpected: {err}");
}

#[test]
fn json_rejects_string() {
    Format::Json.check(&json!({ "verdict": "pass" })).unwrap();
    let err = Format::Json.check(&json!("nope")).unwrap_err();
    assert!(err.contains("not a JSON object"), "unexpected: {err}");
}

#[test]
fn reject_values() {
    verdict_schema().check(&json!({ "verdict": "pass" })).unwrap();
    let err = verdict_schema().check(&json!({ "other": 1 })).unwrap_err();
    assert!(err.contains("does not conform to schema `verdict`"), "unexpected: {err}");
    let err = verdict_schema().check(&json!(42)).unwrap_err();
    assert!(err.contains("does not conform to schema `verdict`"), "unexpected: {err}");
}

#[test]
fn schema_error_path() {
    let format = Format::Schema(Schema {
        name: "report".to_owned(),
        schema: json!({
            "type": "object",
            "properties": { "ui-surface": { "type": "object" } },
        })
        .to_string(),
    });
    let root = format.check(&json!([])).unwrap_err();
    assert!(root.contains("at root"), "unexpected: {root}");
    let nested = format.check(&json!({ "ui-surface": [] })).unwrap_err();
    assert!(nested.contains("/ui-surface"), "unexpected: {nested}");
    assert_ne!(root, nested);
}

#[test]
fn invalid_schema() {
    let mut request = request(vec![message(Role::User, "hi")]);
    request.format = Format::Schema(Schema {
        name: "verdict".to_owned(),
        schema: "not json".to_owned(),
    });
    let err = request.validate().unwrap_err();
    assert!(matches!(err, Error::InvalidRequest(m) if m.contains("not valid JSON")));

    request.format = Format::Schema(Schema {
        name: "verdict".to_owned(),
        schema: r#"{"type":"nonsense"}"#.to_owned(),
    });
    let err = request.validate().unwrap_err();
    assert!(matches!(err, Error::InvalidRequest(m) if m.contains("valid JSON Schema")));
}

#[test]
fn reserved_tool_name() {
    let mut request = request(vec![message(Role::User, "hi")]);
    request.tools.push(Tool::Function(Function {
        name: "read".to_owned(),
        description: "shadow a host-injected tool".to_owned(),
        parameters: "{}".to_owned(),
    }));
    let err = request.validate().unwrap_err();
    assert!(matches!(err, Error::InvalidRequest(m) if m.contains("reserved tool name")));
}

#[test]
fn empty_request() {
    let err = request(vec![]).validate().unwrap_err();
    assert!(matches!(err, Error::InvalidRequest(m) if m == "empty request"));

    let err = request(vec![message(Role::User, "   ")]).validate().unwrap_err();
    assert!(matches!(err, Error::InvalidRequest(m) if m == "empty request"));
}

#[test]
fn invalid_function_parameters() {
    let mut request = request(vec![message(Role::User, "hi")]);
    request.tools.push(Tool::Function(Function {
        name: "lookup".to_owned(),
        description: "look something up".to_owned(),
        parameters: "not json".to_owned(),
    }));
    let err = request.validate().unwrap_err();
    assert!(matches!(err, Error::InvalidRequest(m) if m.contains("`lookup`")));
}

#[test]
fn invalid_format_schema() {
    let mut request = request(vec![message(Role::User, "hi")]);
    request.format = Format::Schema(Schema {
        name: "verdict".to_owned(),
        schema: "not json".to_owned(),
    });
    let err = request.validate().unwrap_err();
    assert!(matches!(err, Error::InvalidRequest(_)));
}

#[test]
fn echo_text() {
    let request = request(vec![message(Role::User, "first"), message(Role::User, "second")]);
    let reply = pipeline(&ModelDefault, request).unwrap();
    assert_eq!(reply.answer, "second");
    assert!(reply.usage.is_none());
}

#[test]
fn echo_json() {
    let mut request = request(vec![message(Role::User, "hi")]);
    request.format = Format::Json;
    let reply = pipeline(&ModelDefault, request).unwrap();
    assert_eq!(reply.answer, r#"{"echo":"hi"}"#);
}

#[test]
fn echo_schema() {
    let mut request = request(vec![message(Role::User, "hi")]);
    request.format = verdict_schema();
    let err = pipeline(&ModelDefault, request).unwrap_err();
    assert!(matches!(err, Error::Backend(m) if m.contains("cannot satisfy format::schema")));
}

#[test]
fn validate_before_complete() {
    let err = pipeline(&Unreached, request(vec![])).unwrap_err();
    assert!(matches!(err, Error::InvalidRequest(m) if m == "empty request"));
}

#[test]
fn canned_schema_answer() {
    let mut request = request(vec![message(Role::User, "judge")]);
    request.format = verdict_schema();
    let reply = pipeline(&Canned(json!({ "verdict": "pass" })), request).unwrap();
    assert_eq!(reply.answer, r#"{"verdict":"pass"}"#);
}

#[test]
fn canned_answer_rejected() {
    let mut request = request(vec![message(Role::User, "judge")]);
    request.format = verdict_schema();
    let err = pipeline(&Canned(json!({ "other": 1 })), request).unwrap_err();
    assert!(
        matches!(err, Error::InvalidAnswer(m) if m.contains("does not conform to schema `verdict`"))
    );
}

// Mirror of the `create` binding's order — validate, complete, project —
// minus the session plumbing, which needs a wasmtime store. Pins the
// guest-visible contract without instantiating a guest.
fn pipeline(backend: &impl WasiModelCtx, request: Request) -> Result<Reply, Error> {
    request.validate()?;
    let format = request.format.clone();
    let answer = block_on(backend.complete(request, Arc::new(NoTools)))?;
    answer.project(&format)
}

// No tier-1 scenario grants tools or a workspace; any call is a contract
// violation, so the stub fails loud.
#[derive(Debug)]
struct NoTools;

impl ToolHost for NoTools {
    fn call_tool(&self, name: String, _arguments: String) -> FutureResult<ToolOutcome> {
        panic!("unexpected tool call `{name}`")
    }

    fn read(&self, path: String) -> FutureResult<Vec<u8>> {
        panic!("unexpected workspace read `{path}`")
    }

    fn list(&self, path: String) -> FutureResult<Vec<DirEntry>> {
        panic!("unexpected workspace list `{path}`")
    }

    fn write(&self, path: String, _bytes: Vec<u8>) -> FutureResult<()> {
        panic!("unexpected workspace write `{path}`")
    }
}

// An invalid request must be rejected before the backend runs.
#[derive(Debug)]
struct Unreached;

impl WasiModelCtx for Unreached {
    fn complete(&self, _request: Request, _tools: Arc<dyn ToolHost>) -> FutureResult<Answer> {
        panic!("invalid request reached the backend")
    }
}

// The canned backend from the testing policy: answers every completion with
// one fixed value, so (unlike the echo) it can satisfy `format::schema`.
#[derive(Clone, Debug)]
struct Canned(Value);

impl WasiModelCtx for Canned {
    fn complete(&self, _request: Request, _tools: Arc<dyn ToolHost>) -> FutureResult<Answer> {
        let answer = Answer {
            value: self.0.clone(),
            usage: None,
            transcript: None,
        };
        async move { Ok(answer) }.boxed()
    }
}

fn verdict_schema() -> Format {
    Format::Schema(Schema {
        name: "verdict".to_owned(),
        schema: json!({
            "type": "object",
            "properties": { "verdict": { "type": "string" } },
            "required": ["verdict"],
        })
        .to_string(),
    })
}

fn report_schema() -> Format {
    Format::Schema(Schema {
        name: "phase-report".to_owned(),
        schema: json!({
            "type": "object",
            "properties": {
                "outcome": { "type": "string" },
                "source": { "type": "string" },
            },
            "required": ["outcome", "source"],
        })
        .to_string(),
    })
}

const fn request(messages: Vec<Message>) -> Request {
    Request {
        model: None,
        system: None,
        messages,
        generation: None,
        format: Format::Text,
        tools: vec![],
        grants: Grants { workspace: None },
    }
}

fn message(role: Role, content: &str) -> Message {
    Message {
        role,
        content: content.to_owned(),
    }
}
