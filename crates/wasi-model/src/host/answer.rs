//! Answer parsing, validation, projection, and repair.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::host::Error;
use crate::host::generated::omnia::model::completion::{
    Format, Reply, Schema, Usage as ReplyUsage,
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

impl From<Usage> for ReplyUsage {
    fn from(usage: Usage) -> Self {
        Self {
            input_tokens: usage.input_tokens,
            output_tokens: usage.output_tokens,
            reasoning_tokens: usage.reasoning_tokens,
        }
    }
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
            usage: self.usage.map(Into::into),
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

    /// Parse and validate a model's text turn.
    ///
    /// # Errors
    ///
    /// Returns a repair reason when the text does not match this format.
    pub fn parse(&self, text: &str) -> Result<Value, String> {
        let value = self.parse_candidate(text)?;
        self.check(&value)?;
        Ok(value)
    }

    /// Parse the best candidate even when it does not pass validation.
    ///
    /// This is only needed when a backend has exhausted its repair budget and
    /// must return the candidate to the host's authoritative answer gate.
    ///
    /// # Errors
    ///
    /// Returns a repair reason when the text contains no candidate.
    pub fn parse_candidate(&self, text: &str) -> Result<Value, String> {
        match self {
            Self::Text => Ok(Value::String(text.to_owned())),
            Self::Json | Self::Schema(_) => {
                let mut last = None;
                for json in maybe_json(text) {
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
            Self::Text | Self::Json => Ok(()),
            Self::Schema(spec) => {
                let validator = schema_validator(spec)?;
                validator.iter_errors(value).next().map_or(Ok(()), |error| {
                    let path = error.instance_path().as_str();
                    let at = if path.is_empty() { "root" } else { path };
                    Err(format!(
                        "answer does not conform to schema `{}`: {error} at {at}",
                        spec.name
                    ))
                })
            }
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

    pub(super) fn validate_definition(&self) -> Result<(), String> {
        match self {
            Self::Schema(spec) => schema_validator(spec).map(drop),
            Self::Text | Self::Json => Ok(()),
        }
    }
}

fn schema_validator(spec: &Schema) -> Result<jsonschema::Validator, String> {
    let schema: Value = serde_json::from_str(&spec.schema)
        .map_err(|error| format!("format schema is not valid JSON: {error}"))?;
    jsonschema::validator_for(&schema)
        .map_err(|error| format!("format schema is not a valid JSON Schema: {error}"))
}

fn maybe_json(text: &str) -> Vec<Value> {
    // try to parse the whole text as a single JSON value
    let text = text.trim();
    if let Ok(value) = serde_json::from_str(text) {
        return vec![value];
    }

    // extract values from "```" fences: fence bodies are the odd-indexed
    // chunks between the delimiters, minus their language-tag line
    let mut values = Vec::new();
    for body in text.split("```").skip(1).step_by(2) {
        let body = body.split_once('\n').map_or(body, |(_tag, body)| body);
        if let Ok(value) = serde_json::from_str(body.trim()) {
            values.push(value);
        }
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

    use crate::host::generated::omnia::model::completion::{Format, Schema};

    #[test]
    fn whole_json_document() {
        assert_eq!(
            Format::Json.parse(r#"{"verdict":"pass"}"#).unwrap(),
            json!({ "verdict": "pass" })
        );
        let err = Format::Json.parse("not json").unwrap_err();
        assert!(err.contains("does not contain JSON"), "unexpected: {err}");
    }

    #[test]
    fn fenced_json() {
        let fenced = "```json\n{\"verdict\":\"pass\"}\n```";
        assert_eq!(verdict_schema().parse(fenced).unwrap(), json!({ "verdict": "pass" }));
    }

    #[test]
    fn json_after_preamble() {
        let text = "Done.\n{\"verdict\":\"pass\"}\n";
        assert_eq!(Format::Json.parse(text).unwrap(), json!({ "verdict": "pass" }));
    }

    #[test]
    fn later_matching_candidate() {
        let text = "findings: []\n{\"outcome\":\"completed\",\"source\":\"model-assisted\"}";
        assert_eq!(
            report_schema().parse(text).unwrap(),
            json!({ "outcome": "completed", "source": "model-assisted" })
        );
    }

    #[test]
    fn invalid_fence_before_matching_candidate() {
        let text = "```json\n[]\n```\n{\"verdict\":\"pass\"}";
        assert_eq!(verdict_schema().parse(text).unwrap(), json!({ "verdict": "pass" }));
    }

    #[test]
    fn invalid_candidate_survives_for_host_gate() {
        let format = report_schema();
        format.parse("[]").unwrap_err();
        let value = format.parse_candidate("[]").unwrap();
        assert_eq!(value, json!([]));

        let err = format.check(&value).unwrap_err();
        assert!(err.contains("does not conform to schema `phase-report`"), "unexpected: {err}");
        assert!(err.contains("at root"), "unexpected: {err}");
    }

    #[test]
    fn embedded_json_is_not_a_matching_candidate() {
        let format = verdict_schema();
        let fenced = r#"{"note":"```json\n{\"verdict\":\"pass\"}\n```"}"#;
        let value = format.parse_candidate(fenced).unwrap();
        assert_eq!(value, json!({ "note": "```json\n{\"verdict\":\"pass\"}\n```" }));
        assert!(format.check(&value).is_err());

        let quoted = r#""use {\"verdict\":\"pass\"} as the answer""#;
        let value = format.parse_candidate(quoted).unwrap();
        assert_eq!(value, json!("use {\"verdict\":\"pass\"} as the answer"));
        assert!(format.check(&value).is_err());
    }

    #[test]
    fn text_rejects_object() {
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
    fn schema_rejects_nonconforming_values() {
        verdict_schema().check(&json!({ "verdict": "pass" })).unwrap();
        let err = verdict_schema().check(&json!({ "other": 1 })).unwrap_err();
        assert!(err.contains("does not conform to schema `verdict`"), "unexpected: {err}");
        let err = verdict_schema().check(&json!(42)).unwrap_err();
        assert!(err.contains("does not conform to schema `verdict`"), "unexpected: {err}");
    }

    #[test]
    fn schema_error_identifies_instance_path() {
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
}
