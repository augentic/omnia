//! Shared helpers for the guest scenario programs in `programs/<capability>/`.

use omnia_sdk::model::{Function, Message, Tool, completion, types};
use schemars::JsonSchema;
use serde::Deserialize;

/// The `verdict` JSON Schema several scenarios request.
pub const VERDICT_SCHEMA: &str =
    r#"{"type":"object","properties":{"verdict":{"type":"string"}},"required":["verdict"]}"#;

/// The typed answer the `check_*` scenarios ask for.
#[derive(Debug, Deserialize, JsonSchema, PartialEq, Eq)]
pub struct Verdict {
    /// `pass` or `fail`.
    pub verdict: String,
    /// What was wrong, when `fail`.
    pub findings: Vec<String>,
}

impl Verdict {
    /// The check every `check_*` scenario applies: a `pass` with no findings.
    ///
    /// # Errors
    ///
    /// Returns the findings a candidate must resolve.
    pub fn passing(&self) -> Result<(), Vec<String>> {
        let mut findings = Vec::new();
        if self.verdict != "pass" {
            findings.push("verdict must be `pass`".to_owned());
        }
        if !self.findings.is_empty() {
            findings.push("findings must be empty".to_owned());
        }
        if findings.is_empty() { Ok(()) } else { Err(findings) }
    }
}

/// One user chat turn.
#[must_use]
pub fn user(content: &str) -> Message {
    Message::user(content)
}

/// A declared `lookup` function tool.
#[must_use]
pub fn lookup() -> Tool {
    Tool::Function(
        Function::builder().name("lookup").description("test lookup").parameters("{}").build(),
    )
}

/// The raw-bindings `lookup` tool.
#[must_use]
pub fn raw_lookup() -> types::Tool {
    types::Tool {
        name: "lookup".to_owned(),
        description: Some("test lookup".to_owned()),
        input_schema: "{}".to_owned(),
    }
}

/// A minimal raw-bindings request — one `hi` user turn, `format::text` —
/// with the given tools and grants.
#[must_use]
pub fn raw_request(tools: Vec<types::Tool>, grants: types::Grants) -> completion::Request {
    completion::Request {
        system: None,
        messages: vec![types::Message {
            role: types::Role::User,
            content: "hi".to_owned(),
        }],
        generation: None,
        format: types::Format::Text,
        tools,
        grants,
        check: false,
    }
}

/// The deployment's `default` model, opened over the raw bindings.
///
/// # Panics
///
/// Panics when the host refuses to open it.
pub async fn raw_model() -> completion::Model {
    completion::open("default".to_owned()).await.expect("open the default model")
}
