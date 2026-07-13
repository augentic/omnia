//! Public fixture-replay behavior.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use futures::executor::block_on;
use omnia_guest::model::{Error, Format, Message, Model, Request, Role, SchemaFormat, Usage};
use omnia_testkit::model::{Recorder, Replay, Scripted};
use serde_json::{Value, json};

#[test]
fn success_and_usage() {
    let fixtures = Fixtures::new();
    fixtures.write(
        "success.json",
        &fixture(
            &text_key("hello"),
            &json!("world"),
            Some(&json!({
                "input_tokens": 4,
                "output_tokens": 2,
                "reasoning_tokens": 1,
            })),
        ),
    );
    let replay = Replay::from_dir(fixtures.path()).unwrap();

    let output = complete(&replay, text_request("hello")).unwrap();
    assert_eq!(output.answer, "world");
    assert_eq!(
        output.usage,
        Some(Usage {
            input_tokens: 4,
            output_tokens: 2,
            reasoning_tokens: Some(1),
        })
    );
}

#[test]
fn miss() {
    let fixtures = Fixtures::new();
    fixtures.write("known.json", &fixture(&text_key("known"), &json!("answer"), None));
    let replay = Replay::from_dir(fixtures.path()).unwrap();

    let error = complete(&replay, text_request("unknown")).unwrap_err();
    assert!(matches!(error, Error::Backend(detail) if detail == "no replay fixture for request"));
}

#[test]
fn malformed_fixture() {
    let fixtures = Fixtures::new();
    std::fs::write(fixtures.path().join("broken.json"), b"{").unwrap();

    let error = Replay::from_dir(fixtures.path()).unwrap_err();
    assert!(error.to_string().contains("parsing fixture"));
}

#[test]
fn missing_directory_is_empty() {
    let fixtures = Fixtures::new();
    let replay = Replay::from_dir(fixtures.path().join("absent")).unwrap();

    let error = complete(&replay, text_request("unknown")).unwrap_err();
    assert!(matches!(error, Error::Backend(detail) if detail == "no replay fixture for request"));
}

#[test]
fn schema_validation() {
    let fixtures = Fixtures::new();
    let schema = r#"{"type":"object","required":["verdict"]}"#;
    fixtures.write(
        "schema.json",
        &fixture(&schema_key("review", schema), &json!({"other": true}), None),
    );
    let replay = Replay::from_dir(fixtures.path()).unwrap();

    let error = complete(&replay, schema_request("review", schema)).unwrap_err();
    assert!(
        matches!(error, Error::InvalidAnswer(detail) if detail.contains("does not conform to schema `result`"))
    );
}

#[test]
fn malformed_schema() {
    let fixtures = Fixtures::new();
    let replay = Replay::from_dir(fixtures.path()).unwrap();

    let error = complete(&replay, schema_request("review", "not json")).unwrap_err();
    assert!(
        matches!(error, Error::InvalidRequest(detail) if detail.contains("format schema is not valid JSON"))
    );
}

#[test]
fn recorder_roundtrip() {
    let fixtures = Fixtures::new();
    let dir = fixtures.path().join("recorded");

    // Record a scripted completion, then replay the same request from the
    // rows the recorder wrote — pins the record and replay key derivations
    // to each other.
    let recorder = Recorder::new(Scripted::reply("world"), &dir);
    let live_reply = complete(&recorder, text_request("hello")).unwrap();
    assert_eq!(live_reply.answer, "world");

    let replay = Replay::from_dir(&dir).unwrap();
    assert_eq!(complete(&replay, text_request("hello")).unwrap().answer, "world");
    let error = complete(&replay, text_request("other")).unwrap_err();
    assert!(matches!(error, Error::Backend(detail) if detail == "no replay fixture for request"));
}

fn complete(model: &impl Model, request: Request) -> Result<omnia_guest::model::Reply, Error> {
    block_on(model.create(request))
}

fn text_request(content: &str) -> Request {
    Request {
        messages: vec![Message {
            role: Role::User,
            content: content.to_owned(),
        }],
        ..Request::default()
    }
}

fn schema_request(content: &str, schema: &str) -> Request {
    Request {
        format: Format::Schema(SchemaFormat {
            name: "result".to_owned(),
            schema: schema.to_owned(),
        }),
        ..text_request(content)
    }
}

fn text_key(content: &str) -> Value {
    key(content, &json!({"kind": "text"}))
}

fn schema_key(content: &str, schema: &str) -> Value {
    key(
        content,
        &json!({
            "kind": "schema",
            "schema": {
                "name": "result",
                "schema": schema,
            },
        }),
    )
}

fn key(content: &str, format: &Value) -> Value {
    json!({
        "model": null,
        "system": null,
        "messages": [{
            "role": "user",
            "content": content,
        }],
        "generation": null,
        "format": format,
        "tools": [],
        "grants": {
            "references": null,
            "verify": [],
        },
    })
}

fn fixture(key_request: &Value, answer: &Value, usage: Option<&Value>) -> Value {
    json!({
        "key_request": key_request,
        "answer": answer,
        "usage": usage,
        "transcript": null,
    })
}

struct Fixtures {
    path: PathBuf,
}

impl Fixtures {
    fn new() -> Self {
        static SEQUENCE: AtomicU64 = AtomicU64::new(0);
        let sequence = SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir()
            .join(format!("omnia-testkit-model-{}-{sequence}", std::process::id()));
        std::fs::create_dir(&path).unwrap();
        Self { path }
    }

    fn path(&self) -> &Path {
        &self.path
    }

    fn write(&self, name: &str, fixture: &Value) {
        std::fs::write(self.path.join(name), serde_json::to_vec_pretty(fixture).unwrap()).unwrap();
    }
}

impl Drop for Fixtures {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}
