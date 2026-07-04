//! Answer parsing, validation, and repair helpers shared by the host gate and
//! backends.
//!
//! Backends drive their repair loops with [`parse_answer`] / [`check_answer`] /
//! [`repair_instruction`]; the host `create` binding re-validates with
//! [`Answer::check`] as the single authority, so backend self-checks only
//! decide whether to spend a repair turn.

use serde_json::Value;

use crate::host::Error;
use crate::host::generated::omnia::model::completion::Format;
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
}

/// Interpret a model's text turn as the answer value for the requested format:
/// `text` wraps the string verbatim; JSON formats must parse (a wrapping
/// Markdown code fence is stripped first).
///
/// # Errors
///
/// Returns the reason the text does not parse for the format, suitable for a
/// repair turn.
pub fn parse_answer(text: &str, format: &Format) -> Result<Value, String> {
    match format {
        Format::Text => Ok(Value::String(text.to_owned())),
        Format::Json | Format::Schema(_) => serde_json::from_str::<Value>(strip_code_fence(text))
            .map_err(|e| format!("the answer was not valid JSON: {e}")),
    }
}

/// Validate an answer value against the requested format: `text` must be a
/// string, `json` an object, and `schema` must validate against the schema
/// document.
///
/// # Errors
///
/// Returns the first validation failure, suitable for a repair turn.
pub fn check_answer(value: &Value, format: &Format) -> Result<(), String> {
    match format {
        Format::Text if !value.is_string() => Err("answer is not a JSON string".to_owned()),
        Format::Json if !value.is_object() => Err("answer is not a JSON object".to_owned()),
        Format::Schema(spec) => {
            let schema: Value = serde_json::from_str(&spec.schema)
                .map_err(|e| format!("format schema is not valid JSON: {e}"))?;
            let validator = jsonschema::validator_for(&schema)
                .map_err(|e| format!("format schema is not a valid JSON Schema: {e}"))?;
            validator.iter_errors(value).next().map_or(Ok(()), |error| {
                Err(format!("answer does not conform to schema `{}`: {error}", spec.name))
            })
        }
        _ => Ok(()),
    }
}

/// The correction instruction a backend appends (with the rejected answer)
/// before spending a repair turn.
#[must_use]
pub fn repair_instruction(reason: &str) -> String {
    format!(
        "Your previous answer did not satisfy the required response format ({reason}). Reply \
         again with only the corrected answer and nothing else."
    )
}

// Strip a wrapping Markdown code fence (```json ... ```), if present.
fn strip_code_fence(text: &str) -> &str {
    let trimmed = text.trim();
    let Some(rest) = trimmed.strip_prefix("```") else {
        return trimmed;
    };
    let body = rest.split_once('\n').map_or(rest, |(_, body)| body).trim();
    body.strip_suffix("```").unwrap_or(body).trim()
}

impl Answer {
    /// Validate an answer against the request's `format`.
    ///
    /// # Errors
    ///
    /// Returns an error when the answer does not match the requested format.
    pub fn check(value: &Value, format: &Format) -> Result<(), Error> {
        check_answer(value, format).map_err(Error::InvalidAnswer)
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{check_answer, parse_answer, repair_instruction};
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
        assert_eq!(parse_answer("hello", &Format::Text).unwrap(), json!("hello"));
    }

    #[test]
    fn json_must_parse() {
        assert_eq!(
            parse_answer(r#"{"verdict":"pass"}"#, &Format::Json).unwrap(),
            json!({ "verdict": "pass" })
        );
        let err = parse_answer("not json", &Format::Json).unwrap_err();
        assert!(err.contains("not valid JSON"), "unexpected: {err}");
    }

    #[test]
    fn code_fence_stripped() {
        let fenced = "```json\n{\"verdict\":\"pass\"}\n```";
        assert_eq!(parse_answer(fenced, &verdict_schema()).unwrap(), json!({ "verdict": "pass" }));
    }

    #[test]
    fn json_string() {
        check_answer(&json!("hi"), &Format::Text).unwrap();
        let err = check_answer(&json!({ "a": 1 }), &Format::Text).unwrap_err();
        assert!(err.contains("not a JSON string"), "unexpected: {err}");
    }

    #[test]
    fn json_object() {
        check_answer(&json!({ "verdict": "pass" }), &Format::Json).unwrap();
        let err = check_answer(&json!("nope"), &Format::Json).unwrap_err();
        assert!(err.contains("not a JSON object"), "unexpected: {err}");
    }

    #[test]
    fn schema_enforced() {
        check_answer(&json!({ "verdict": "pass" }), &verdict_schema()).unwrap();
        let err = check_answer(&json!({ "other": 1 }), &verdict_schema()).unwrap_err();
        assert!(err.contains("does not conform to schema `verdict`"), "unexpected: {err}");
        let err = check_answer(&json!(42), &verdict_schema()).unwrap_err();
        assert!(err.contains("does not conform to schema `verdict`"), "unexpected: {err}");
    }

    #[test]
    fn repair_carries_reason() {
        let text = repair_instruction("answer is not a JSON object");
        assert!(text.contains("answer is not a JSON object"));
    }
}
