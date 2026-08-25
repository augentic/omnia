//! Answer parsing, validation, projection, and repair behavior shared by the
//! host gate and backends.

use serde_json::Value;

use crate::host::Error;
use crate::host::generated::omnia::model::completion::{Format, Reply, Usage};
use crate::host::resource::Answer;

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
            usage: self.usage.map(|usage| Usage {
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
                for value in json_values(text) {
                    if self.check(&value).is_ok() {
                        return Ok(value);
                    }
                    last = Some(value);
                }
                last.ok_or_else(|| "answer is not valid JSON".into())
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

// Complete JSON values in `text`. A whole-text parse is the only candidate so
// fence / brace scans cannot lift fragments out of string contents. Otherwise:
// each fenced body, then each `{` / `[` slice.
fn json_values(text: &str) -> Vec<Value> {
    let trimmed = text.trim();
    if let Ok(value) = serde_json::from_str(trimmed) {
        return vec![value];
    }
    fenced_bodies(trimmed)
        .into_iter()
        .filter_map(|body| serde_json::from_str(body.trim()).ok())
        .chain(sliced_json(trimmed))
        .collect()
}

// Bodies of every Markdown fence in `text`; an unterminated fence yields the
// remainder.
fn fenced_bodies(text: &str) -> Vec<&str> {
    let mut bodies = Vec::new();
    let mut rest = text;
    while let Some(start) = rest.find("```") {
        let after = &rest[start + 3..];
        let body = after.split_once('\n').map_or(after, |(_, body)| body);
        if let Some(end) = body.find("```") {
            bodies.push(&body[..end]);
            rest = &body[end + 3..];
        } else {
            bodies.push(body);
            break;
        }
    }
    bodies
}

// Complete JSON values starting at each `{` or `[`, continuing from the
// deserializer offset so nested brackets are not re-offered as roots.
fn sliced_json(text: &str) -> Vec<Value> {
    let mut values = Vec::new();
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

// Unit tests by design: answer extraction/repair is a pure parser; the model
// ABI scenarios cover the request/answer round-trip through a guest.
#[cfg(test)]
mod tests {
    use serde_json::json;

    use crate::host::generated::omnia::model::completion::{Format, Schema};

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

    fn phase_report_schema() -> Format {
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

    #[test]
    fn json_must_parse() {
        assert_eq!(
            Format::Json.parse(r#"{"verdict":"pass"}"#).unwrap(),
            json!({ "verdict": "pass" })
        );
        let err = Format::Json.parse("not json").unwrap_err();
        assert!(err.contains("not valid JSON"), "unexpected: {err}");
    }

    #[test]
    fn code_fence_stripped() {
        let fenced = "```json\n{\"verdict\":\"pass\"}\n```";
        assert_eq!(verdict_schema().parse(fenced).unwrap(), json!({ "verdict": "pass" }));
    }

    #[test]
    fn preamble_then_raw_object() {
        let text = "Done.\n{\"verdict\":\"pass\"}\n";
        assert_eq!(Format::Json.parse(text).unwrap(), json!({ "verdict": "pass" }));
    }

    #[test]
    fn preamble_array_then_phase_report() {
        let text = "findings: []\n{\"outcome\":\"completed\",\"source\":\"model-assisted\"}";
        assert_eq!(
            phase_report_schema().parse(text).unwrap(),
            json!({ "outcome": "completed", "source": "model-assisted" })
        );
    }

    #[test]
    fn fenced_array_then_raw_object() {
        let text = "```json\n[]\n```\n{\"verdict\":\"pass\"}";
        assert_eq!(verdict_schema().parse(text).unwrap(), json!({ "verdict": "pass" }));
    }

    #[test]
    fn whole_text_array_is_schema_error() {
        let format = phase_report_schema();
        let value = format.parse("[]").unwrap();
        assert_eq!(value, json!([]));
        let err = format.check(&value).unwrap_err();
        assert!(err.contains("does not conform to schema `phase-report`"), "unexpected: {err}");
        assert!(err.contains("at root"), "unexpected: {err}");
    }

    #[test]
    fn whole_json_not_mined() {
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
    fn json_string() {
        Format::Text.check(&json!("hi")).unwrap();
        let err = Format::Text.check(&json!({ "a": 1 })).unwrap_err();
        assert!(err.contains("not a JSON string"), "unexpected: {err}");
    }

    #[test]
    fn json_object() {
        Format::Json.check(&json!({ "verdict": "pass" })).unwrap();
        let err = Format::Json.check(&json!("nope")).unwrap_err();
        assert!(err.contains("not a JSON object"), "unexpected: {err}");
    }

    #[test]
    fn schema_enforced() {
        verdict_schema().check(&json!({ "verdict": "pass" })).unwrap();
        let err = verdict_schema().check(&json!({ "other": 1 })).unwrap_err();
        assert!(err.contains("does not conform to schema `verdict`"), "unexpected: {err}");
        let err = verdict_schema().check(&json!(42)).unwrap_err();
        assert!(err.contains("does not conform to schema `verdict`"), "unexpected: {err}");
    }

    #[test]
    fn check_names_instance_path() {
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
}
