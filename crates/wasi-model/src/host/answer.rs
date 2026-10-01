//! The backend's answer and the candidate extraction it may apply to a
//! model's final text. Nothing here validates: acceptance is the guest's
//! `check`, or nothing at all when the request declares none.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::host::generated::omnia::model::completion::{Format, Usage as ReplyUsage};

/// A backend's result: the answer text, optional usage, and transcript.
///
/// Host-only — the guest sees a `reply` carrying `answer` and `usage`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Answer {
    /// The text the guest's check accepted, or the model's final text.
    pub answer: String,
    /// Token accounting the backend reported, surfaced to the guest as `reply.usage`.
    pub usage: Option<Usage>,
    /// Optional tool-call transcript the backend captured.
    pub transcript: Option<Transcript>,
}

impl From<String> for Answer {
    fn from(answer: String) -> Self {
        Self {
            answer,
            usage: None,
            transcript: None,
        }
    }
}

impl From<&str> for Answer {
    fn from(answer: &str) -> Self {
        answer.to_owned().into()
    }
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
/// replay. Host-only; it never crosses the WIT boundary. Empty when the
/// backend captured no tool turns.
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

impl std::fmt::Display for Format {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Text => "text",
            Self::Json => "json",
            Self::Schema(_) => "schema",
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

    /// The candidate answer in a model's final text and how it was read:
    /// the text itself for `text`; for `json` and `schema`, the JSON value
    /// the text holds, as [`Reading`] describes. A courtesy for providers
    /// that wrap JSON in prose, never a gate — the guest's check decides.
    #[must_use]
    pub fn candidate(&self, text: &str) -> Candidate {
        match self {
            Self::Text => Candidate {
                text: text.to_owned(),
                reading: Reading::Text,
            },
            Self::Json | Self::Schema(_) => Candidate::from_json(text),
        }
    }
}

/// What a backend puts to the guest's check from a model's final text, and
/// how it was read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Candidate {
    /// The text as written, or the one JSON value read from it.
    pub text: String,
    /// How `text` was read.
    pub reading: Reading,
}

impl Candidate {
    fn from_json(text: &str) -> Self {
        // the whole reply is one value
        let fault = match serde_json::from_str::<Value>(text) {
            Ok(value) => {
                return Self {
                    text: value.to_string(),
                    reading: Reading::Value,
                };
            }
            Err(fault) => fault,
        };
        let unparsed = || Self {
            text: text.to_owned(),
            reading: Reading::Unparsed {
                fault: fault.to_string(),
            },
        };

        // a document that opens the reply and does not parse is malformed,
        // not wrapped: its members are not the answer
        if text.trim_start().starts_with(['{', '[']) && leading(text).is_none() {
            return unparsed();
        }

        // what the prose wraps: one value, or the largest of several
        let found: Vec<String> = maybe_json(text).iter().map(ToString::to_string).collect();
        let of = found.len();
        match found.into_iter().max_by_key(String::len) {
            None => unparsed(),
            Some(text) if of == 1 => Self {
                text,
                reading: Reading::Value,
            },
            Some(text) => Self {
                text,
                reading: Reading::Largest { of },
            },
        }
    }

    /// The turn that asks a reply read as no JSON value for one that is,
    /// naming the parser's fault; `None` when a value was read, or the
    /// `text` format asked for none.
    #[must_use]
    pub fn nudge(&self) -> Option<String> {
        let Reading::Unparsed { fault } = &self.reading else {
            return None;
        };
        Some(format!(
            "Your last reply is not one well-formed JSON value ({fault}), so it is not the \
             answer. Reply now with only your final answer as a single JSON value in the shape \
             the prompt asked for, and nothing else."
        ))
    }

    /// The turn that puts the guest's `correction` to the model: led by one
    /// sentence naming the count when the reply held several JSON values and
    /// the largest was the one checked, the correction alone otherwise.
    #[must_use]
    pub fn correction(&self, correction: String) -> String {
        match self.reading {
            Reading::Largest { of } => format!(
                "Your reply held {of} JSON values; the largest was read as your answer and is \
                 the previous answer below. Reply with only your final answer as one JSON \
                 value.\n\n{correction}"
            ),
            _ => correction,
        }
    }
}

/// How a candidate was read from a model's final text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Reading {
    /// As written: the `text` format reads nothing from the reply.
    Text,
    /// The one JSON value the reply holds, whole or wrapped in prose.
    Value,
    /// The largest of the distinct JSON values the reply holds.
    Largest {
        /// How many distinct JSON values the reply held.
        of: usize,
    },
    /// As written, because the reply holds no JSON value or opens one that
    /// does not parse.
    Unparsed {
        /// The parser's word on the whole reply.
        fault: String,
    },
}

// The JSON value that opens `text`, when one does.
fn leading(text: &str) -> Option<Value> {
    serde_json::Deserializer::from_str(text).into_iter::<Value>().next()?.ok()
}

// Every distinct JSON value in `text`, each once: the bodies of "```" fences
// first, then every `{` / `[` slice.
fn maybe_json(text: &str) -> Vec<Value> {
    let mut values: Vec<Value> = Vec::new();
    let mut push = |value: Value| {
        if !values.contains(&value) {
            values.push(value);
        }
    };

    // extract values from "```" fences: fence bodies are the odd-indexed
    // chunks between the delimiters, minus their language-tag line
    for body in text.split("```").skip(1).step_by(2) {
        let body = body.split_once('\n').map_or(body, |(_tag, body)| body);
        if let Ok(value) = serde_json::from_str(body.trim()) {
            push(value);
        }
    }

    // extract values from `{` or `[` slices
    let mut rest = text;
    while let Some(offset) = rest.find(['{', '[']) {
        let mut stream = serde_json::Deserializer::from_str(&rest[offset..]).into_iter::<Value>();
        match stream.next() {
            Some(Ok(value)) => {
                rest = &rest[offset + stream.byte_offset()..];
                push(value);
            }
            Some(Err(_)) | None => rest = &rest[offset + 1..],
        }
    }

    values
}

// `candidate`, the turns it offers and `instruction` are pure; backends
// drive them directly
#[cfg(test)]
mod tests {
    use super::{Candidate, Format, Reading};
    use crate::host::generated::omnia::model::completion::Schema;

    fn verdict_schema() -> Format {
        Format::Schema(Schema {
            name: "verdict".to_owned(),
            schema: r#"{"type":"object"}"#.to_owned(),
        })
    }

    fn candidate(text: &str) -> Candidate {
        verdict_schema().candidate(text)
    }

    fn read(text: &str, reading: Reading) -> Candidate {
        Candidate {
            text: text.to_owned(),
            reading,
        }
    }

    #[test]
    fn text_passthrough() {
        let text = "  plain {not: json}  ";
        assert_eq!(Format::Text.candidate(text), read(text, Reading::Text));
    }

    #[test]
    fn json_document() {
        assert_eq!(
            candidate(r#"{"verdict":"pass"}"#),
            read(r#"{"verdict":"pass"}"#, Reading::Value)
        );
    }

    // a fenced document is one value, not one per way of finding it
    #[test]
    fn fenced_json() {
        assert_eq!(
            candidate("```json\n{\"verdict\":\"pass\"}\n```"),
            read(r#"{"verdict":"pass"}"#, Reading::Value)
        );
    }

    #[test]
    fn json_with_preamble() {
        assert_eq!(
            candidate("Done.\n{\"verdict\":\"pass\"}\n"),
            read(r#"{"verdict":"pass"}"#, Reading::Value)
        );
    }

    #[test]
    fn largest_value_wins() {
        assert_eq!(
            candidate("findings: []\n{\"outcome\":\"completed\"}"),
            read(r#"{"outcome":"completed"}"#, Reading::Largest { of: 2 })
        );
    }

    // a fragment of the answer repeated after it is not the answer
    #[test]
    fn document_then_fragment() {
        let document = r#"{"requirements":[{"scenarios":[],"subject":"a.run"}]}"#;
        assert_eq!(
            candidate(&format!("{document}\n{{\"scenarios\":[],\"subject\":\"a.run\"}}")),
            read(document, Reading::Largest { of: 2 })
        );
    }

    // a worked example before the answer is smaller than the answer
    #[test]
    fn example_then_answer() {
        let text = "For example {\"verdict\":\"fail\"} — and mine:\n{\"findings\":[],\"verdict\":\"pass\"}";
        assert_eq!(
            candidate(text),
            read(r#"{"findings":[],"verdict":"pass"}"#, Reading::Largest { of: 2 })
        );
    }

    // a document cut short is handed back whole, with the parser's fault at
    // its line and column rather than a well-formed member's shape
    #[test]
    fn malformed_document() {
        let truncated = "{\n  \"requirements\": [{\"scenarios\":[],\"subject\":\"a.run\"}],\n";
        let Candidate { text, reading } = candidate(truncated);
        assert_eq!(text, truncated);
        let Reading::Unparsed { fault } = reading else {
            panic!("{reading:?}");
        };
        assert!(fault.contains("line 3"), "{fault}");
    }

    #[test]
    fn no_json() {
        let Candidate { text, reading } = candidate("not json");
        assert_eq!(text, "not json");
        assert!(matches!(reading, Reading::Unparsed { .. }), "{reading:?}");
    }

    // the nudge names the parser's fault; a reply read as a value, or as
    // text, is not nudged
    #[test]
    fn nudge_names_the_fault() {
        let nudge = candidate("Analyzing the claims.").nudge().expect("unparsed");
        assert!(
            nudge.starts_with(
                "Your last reply is not one well-formed JSON value (expected value at line 1 \
                 column 1), so it is not the answer."
            ),
            "{nudge}"
        );
        assert_eq!(candidate(r#"{"verdict":"pass"}"#).nudge(), None);
        assert_eq!(Format::Text.candidate("Analyzing the claims.").nudge(), None);
    }

    // the correction leads with the count only when the largest of several
    // values was the one checked
    #[test]
    fn correction_leads_with_the_count() {
        let correction = "## Previous answer (rejected)\n\n{}".to_owned();
        let led =
            candidate("findings: []\n{\"outcome\":\"completed\"}").correction(correction.clone());
        assert!(
            led.starts_with("Your reply held 2 JSON values; the largest was read as your answer"),
            "{led}"
        );
        assert!(led.ends_with(&format!("one JSON value.\n\n{correction}")), "{led}");
        assert_eq!(candidate(r#"{"verdict":"pass"}"#).correction(correction.clone()), correction);
    }

    #[test]
    fn instruction_per_format() {
        assert!(Format::Text.instruction().contains("plain text"));
        assert!(Format::Json.instruction().contains("JSON object"));
        let schema = verdict_schema().instruction();
        assert!(schema.contains("JSON Schema"), "unexpected: {schema}");
        assert!(schema.contains("object"), "unexpected: {schema}");
    }

    #[test]
    fn format_display() {
        assert_eq!(Format::Text.to_string(), "text");
        assert_eq!(Format::Json.to_string(), "json");
        assert_eq!(verdict_schema().to_string(), "schema");
    }
}
