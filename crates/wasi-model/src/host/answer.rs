//! Answer parsing, validation, projection, and repair behavior shared by the
//! host gate and backends.

use serde::Deserialize as _;
use serde_json::Value;

use crate::host::Error;
use crate::host::generated::omnia::model::completion::{Format, Reply, Usage};
use crate::host::types::Answer;

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
    /// For JSON / schema answers, tolerates a wrapping Markdown fence and —
    /// when the model prepends agent prose — an embedded fence or the first
    /// complete JSON value in the turn. Schema conformance is still enforced
    /// by [`Self::check`].
    ///
    /// # Errors
    ///
    /// Returns a repair reason when the text does not match this format.
    pub fn parse(&self, text: &str) -> Result<Value, String> {
        match self {
            Self::Text => Ok(Value::String(text.to_owned())),
            Self::Json | Self::Schema(_) => {
                into_json(text).map_err(|err| format!("answer is not valid JSON: {err}"))
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
                    Err(format!("answer does not conform to schema `{}`: {error}", spec.name))
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

/// Parse a JSON answer, accepting raw JSON, a leading or embedded Markdown
/// fence, or a JSON value preceded / followed by agent prose.
fn into_json(text: &str) -> Result<Value, serde_json::Error> {
    let trimmed = text.trim();

    // 1. Whole text as-is.
    if let Ok(value) = serde_json::from_str(trimmed) {
        return Ok(value);
    }

    // 2. Fence anywhere (agent preamble + ```json … ```), closed or not.
    if let Some(value) = from_fenced(trimmed) {
        return Ok(value);
    }

    // 3. First complete JSON value at the first `{` or `[`.
    let offset = trimmed.find(['{', '[']).unwrap_or(0);
    // parse one value, ignoring any trailing text after closing `}` or `]`
    let mut de = serde_json::Deserializer::from_str(&trimmed[offset..]);
    Value::deserialize(&mut de)
}

// Body of the first Markdown fence in `text`, if any; an unterminated fence
// yields the remainder of the text.
fn from_fenced(text: &str) -> Option<Value> {
    let start = text.find("```")?;
    let rest = &text[start + 3..];
    let body = rest.split_once('\n').map_or(rest, |(_, body)| body);
    let body = body.find("```").map_or(body, |end| &body[..end]);
    serde_json::from_str(body.trim()).ok()
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
            usage: self.usage.map(|usage| Usage {
                input_tokens: usage.input_tokens,
                output_tokens: usage.output_tokens,
                reasoning_tokens: usage.reasoning_tokens,
            }),
        })
    }
}

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

    #[test]
    fn text_is_verbatim() {
        assert_eq!(Format::Text.parse("hello").unwrap(), json!("hello"));
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
    fn preamble_then_fence() {
        let text = "Let me synthesize.\n```json\n{\"verdict\":\"pass\"}\n```";
        assert_eq!(Format::Json.parse(text).unwrap(), json!({ "verdict": "pass" }));
    }

    #[test]
    fn preamble_then_raw_object() {
        let text = "Done.\n{\"verdict\":\"pass\"}\n";
        assert_eq!(Format::Json.parse(text).unwrap(), json!({ "verdict": "pass" }));
    }

    #[test]
    fn trailing_prose() {
        let text = "{\"verdict\":\"pass\"}\nthanks";
        assert_eq!(Format::Json.parse(text).unwrap(), json!({ "verdict": "pass" }));
    }

    #[test]
    fn still_rejects_non_json() {
        let err = Format::Json.parse("no braces here").unwrap_err();
        assert!(err.contains("not valid JSON"), "unexpected: {err}");
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
    fn repair_carries_reason() {
        let text = Format::Json.repair("answer is not a JSON object");
        assert!(text.contains("answer is not a JSON object"));
    }
}
