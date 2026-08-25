//! Answer parsing, validation, projection, and repair behavior shared by the
//! host gate and backends, plus request prompt shaping (`Display`, MCP grants).

use std::fmt;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::host::Error;
use crate::host::generated::omnia::model::completion::{
    Format, Mcp, Reply, Request, Role, Tool, Usage as ReplyUsage,
};

/// A backend's result: the parsed answer value, optional usage, and transcript.
///
/// Host-only — the guest sees a `reply` whose `answer` is the validated string
/// the `create` binding derives from `value`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Answer {
    /// The parsed JSON answer the backend produced.
    pub value: serde_json::Value,
    /// Token accounting the backend reported, surfaced to the guest as `reply.usage`.
    pub usage: Option<Usage>,
    /// Optional tool-call transcript the backend captured.
    pub transcript: Option<Transcript>,
}

/// Token accounting for one completion. Mirrors the WIT `usage` record; the
/// serde derive lets backends record it alongside the transcript.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Usage {
    /// Prompt tokens consumed.
    pub input_tokens: u32,
    /// Completion tokens produced.
    pub output_tokens: u32,
    /// Reasoning tokens, for models that bill them separately.
    pub reasoning_tokens: Option<u32>,
}

/// The tool-call transcript a backend may capture for diagnostics or future
/// replay. Host-only; it never crosses the WIT boundary. Empty for backends
/// with no tool loop (cursor).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Transcript {
    /// Ordered tool turns the backend drove to reach the answer.
    pub turns: Vec<ToolTurn>,
}

/// One recorded tool interaction within a completion's transcript.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolTurn {
    /// The tool the model called.
    pub tool: String,
    /// The arguments the model supplied.
    pub args: serde_json::Value,
    /// The result the host returned.
    pub result: serde_json::Value,
}

impl Answer {
    /// Validate an answer against the request's `format`.
    ///
    /// # Errors
    ///
    /// Returns an error when the answer does not match the requested format.
    pub fn check(&self, format: &Format) -> Result<(), Error> {
        format.check(&self.value).map_err(Error::InvalidAnswer)
    }

    /// Project this answer to the guest-visible wire reply.
    ///
    /// # Errors
    ///
    /// Returns an error when the answer does not match `format` or cannot be serialized.
    pub fn project(&self, format: &Format) -> Result<Reply, Error> {
        self.check(format)?;

        let text = match (format, &self.value) {
            (Format::Text, Value::String(text)) => text.clone(),
            _ => serde_json::to_string(&self.value)
                .map_err(|error| Error::InvalidAnswer(error.to_string()))?,
        };

        Ok(Reply {
            answer: text,
            usage: self.usage.map(|usage| ReplyUsage {
                input_tokens: usage.input_tokens,
                output_tokens: usage.output_tokens,
                reasoning_tokens: usage.reasoning_tokens,
            }),
        })
    }
}

impl Format {
    /// The final-answer instruction appended to a prompt for backends that
    /// steer output shape through prose rather than a provider `response_format`.
    #[must_use]
    pub fn instruction(&self) -> String {
        match self {
            Self::Schema(spec) => format!(
                "When you are done, reply with only your final answer as a single JSON value \
                 conforming to this JSON Schema, and nothing else:\n{}",
                spec.schema
            ),
            Self::Json => "When you are done, reply with only your final answer as a single JSON \
                           object and nothing else."
                .to_owned(),
            Self::Text => {
                "When you are done, reply with only your final answer as plain text and nothing \
                 else."
                    .to_owned()
            }
        }
    }

    /// Interpret a model's text turn as an answer value.
    ///
    /// For JSON / schema answers, a whole-text JSON value is the only
    /// candidate. Otherwise every complete value in the turn is tried (each
    /// fenced body, then each `{` / `[` slice). The first value that passes
    /// [`Self::check`] wins, so an incidental `[]` in preamble does not hide a
    /// later valid object. If none pass, the last extracted value is returned
    /// so the host gate remains the authority.
    ///
    /// # Errors
    ///
    /// Returns a repair reason when the text does not match this format.
    pub fn parse(&self, text: &str) -> Result<Value, String> {
        match self {
            Self::Text => Ok(Value::String(text.to_owned())),
            Self::Json | Self::Schema(_) => {
                let mut last = None;
                for json in into_json(text) {
                    if self.check(&json).is_ok() {
                        return Ok(json);
                    }
                    last = Some(json);
                }
                last.ok_or_else(|| "answer does not contain JSON".into())
            }
        }
    }

    /// Validate an answer value against this format.
    ///
    /// # Errors
    ///
    /// Returns the first validation failure, suitable for a repair turn.
    pub fn check(&self, value: &Value) -> Result<(), String> {
        match self {
            Self::Text if !value.is_string() => Err("answer is not a JSON string".to_owned()),
            Self::Json if !value.is_object() => Err("answer is not a JSON object".to_owned()),
            Self::Schema(spec) => {
                let schema: Value = serde_json::from_str(&spec.schema)
                    .map_err(|error| format!("format schema is not valid JSON: {error}"))?;
                let validator = jsonschema::validator_for(&schema).map_err(|error| {
                    format!("format schema is not a valid JSON Schema: {error}")
                })?;
                validator.iter_errors(value).next().map_or(Ok(()), |error| {
                    let path = error.instance_path().as_str();
                    let at = if path.is_empty() { "root" } else { path };
                    Err(format!(
                        "answer does not conform to schema `{}`: {error} at {at}",
                        spec.name
                    ))
                })
            }
            _ => Ok(()),
        }
    }

    /// Build the correction instruction for a rejected answer.
    #[must_use]
    pub fn repair(&self, reason: &str) -> String {
        format!(
            "Your previous answer did not satisfy the required response format ({reason}). Reply \
             again with only the corrected answer and nothing else."
        )
    }
}

impl fmt::Display for Role {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::System => "system",
            Self::User => "user",
            Self::Assistant => "assistant",
        })
    }
}

// The single-text-prompt form, for backends that steer output shape through
// prose (see `Format::instruction`): system channel first, then each message
// (non-user turns marked with their role), then the format instruction.
impl fmt::Display for Request {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut sep = if let Some(system) = &self.system {
            f.write_str(system)?;
            "\n\n"
        } else {
            ""
        };
        for message in &self.messages {
            match message.role {
                Role::User => write!(f, "{sep}{}", message.content)?,
                Role::System | Role::Assistant => {
                    write!(f, "{sep}[{}]\n{}", message.role, message.content)?;
                }
            }
            sep = "\n\n";
        }
        write!(f, "{sep}{}", self.format.instruction())
    }
}

impl Request {
    /// The request's MCP server grants, each carrying its own endpoint URL.
    #[must_use]
    pub fn mcp_servers(&self) -> Vec<&Mcp> {
        self.tools
            .iter()
            .filter_map(|tool| match tool {
                Tool::Mcp(grant) => Some(grant),
                Tool::Function(_) => None,
            })
            .collect()
    }
}

fn into_json(text: &str) -> Vec<Value> {
    // try to parse the whole text as a single JSON value
    let text = text.trim();
    if let Ok(value) = serde_json::from_str(text) {
        return vec![value];
    }

    // extract values from "```" fences
    let mut values = Vec::new();
    let mut rest = text;
    while let Some(start) = rest.find("```") {
        let after = &rest[start + 3..];
        let body = after.split_once('\n').map_or(after, |(_, body)| body);
        let end = body.find("```").unwrap_or(body.len());
        if let Ok(value) = serde_json::from_str(body[..end].trim()) {
            values.push(value);
        }
        if end == body.len() {
            break;
        }
        rest = &body[end + 3..];
    }

    // extract values from `{` or `[` slices
    let mut rest = text;
    while let Some(offset) = rest.find(['{', '[']) {
        let mut stream = serde_json::Deserializer::from_str(&rest[offset..]).into_iter::<Value>();
        match stream.next() {
            Some(Ok(value)) => {
                rest = &rest[offset + stream.byte_offset()..];
                values.push(value);
            }
            Some(Err(_)) | None => rest = &rest[offset + 1..],
        }
    }
    values
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use crate::host::generated::omnia::model::completion::{
        Format, Grants, Message, Request, Role, Schema,
    };

    #[test]
    fn json() {
        assert_eq!(
            Format::Json.parse(r#"{"verdict":"pass"}"#).unwrap(),
            json!({ "verdict": "pass" })
        );
        let err = Format::Json.parse("not json").unwrap_err();
        assert!(err.contains("does not contain JSON"), "unexpected: {err}");
    }

    #[test]
    fn fence() {
        let fenced = "```json\n{\"verdict\":\"pass\"}\n```";
        assert_eq!(verdict_schema().parse(fenced).unwrap(), json!({ "verdict": "pass" }));
    }

    #[test]
    fn preamble() {
        let text = "Done.\n{\"verdict\":\"pass\"}\n";
        assert_eq!(Format::Json.parse(text).unwrap(), json!({ "verdict": "pass" }));
    }

    #[test]
    fn incidental() {
        let text = "findings: []\n{\"outcome\":\"completed\",\"source\":\"model-assisted\"}";
        assert_eq!(
            report_schema().parse(text).unwrap(),
            json!({ "outcome": "completed", "source": "model-assisted" })
        );
    }

    #[test]
    fn fenced_array() {
        let text = "```json\n[]\n```\n{\"verdict\":\"pass\"}";
        assert_eq!(verdict_schema().parse(text).unwrap(), json!({ "verdict": "pass" }));
    }

    #[test]
    fn bad_phase_report() {
        let format = report_schema();
        let value = format.parse("[]").unwrap();
        assert_eq!(value, json!([]));

        let err = format.check(&value).unwrap_err();
        assert!(err.contains("does not conform to schema `phase-report`"), "unexpected: {err}");
        assert!(err.contains("at root"), "unexpected: {err}");
    }

    #[test]
    fn bad_verdict() {
        let format = verdict_schema();
        let fenced = r#"{"note":"```json\n{\"verdict\":\"pass\"}\n```"}"#;
        let value = format.parse(fenced).unwrap();
        assert_eq!(value, json!({ "note": "```json\n{\"verdict\":\"pass\"}\n```" }));
        assert!(format.check(&value).is_err());

        let quoted = r#""use {\"verdict\":\"pass\"} as the answer""#;
        let value = format.parse(quoted).unwrap();
        assert_eq!(value, json!("use {\"verdict\":\"pass\"} as the answer"));
        assert!(format.check(&value).is_err());
    }

    #[test]
    fn not_json() {
        Format::Text.check(&json!("hi")).unwrap();
        let err = Format::Text.check(&json!({ "a": 1 })).unwrap_err();
        assert!(err.contains("not a JSON string"), "unexpected: {err}");
    }

    #[test]
    fn not_object() {
        Format::Json.check(&json!({ "verdict": "pass" })).unwrap();
        let err = Format::Json.check(&json!("nope")).unwrap_err();
        assert!(err.contains("not a JSON object"), "unexpected: {err}");
    }

    #[test]
    fn invalid_schema() {
        verdict_schema().check(&json!({ "verdict": "pass" })).unwrap();
        let err = verdict_schema().check(&json!({ "other": 1 })).unwrap_err();
        assert!(err.contains("does not conform to schema `verdict`"), "unexpected: {err}");
        let err = verdict_schema().check(&json!(42)).unwrap_err();
        assert!(err.contains("does not conform to schema `verdict`"), "unexpected: {err}");
    }

    #[test]
    fn invalid_path() {
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

    fn request(system: Option<&str>, messages: Vec<Message>) -> Request {
        Request {
            model: None,
            system: system.map(str::to_owned),
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

    #[test]
    fn display_prompt() {
        let request = request(
            Some("sys"),
            vec![
                message(Role::User, "hi"),
                message(Role::Assistant, "ack"),
                message(Role::System, "note"),
            ],
        );
        let expected = format!(
            "sys\n\nhi\n\n[assistant]\nack\n\n[system]\nnote\n\n{}",
            Format::Text.instruction()
        );
        assert_eq!(request.to_string(), expected);
    }
}
