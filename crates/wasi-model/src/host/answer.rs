//! The backend's answer and the candidate extraction it may apply to a
//! model's final text. Nothing here validates: acceptance is the guest's
//! `check`, or nothing at all when the request declares none.

use serde_json::Value;

use crate::host::generated::omnia::model::completion::{Format, Usage};

/// A backend's result: the answer text, optional usage, and transcript.
///
/// Host-only — the guest sees a `reply` carrying `answer` and `usage`.
#[derive(Clone, Debug)]
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

/// The tool-call transcript a backend may capture for diagnostics. Host-only;
/// it never crosses the WIT boundary. Empty when the backend captured no
/// tool turns.
#[derive(Clone, Debug, Default)]
pub struct Transcript {
    /// Ordered tool turns the backend drove to reach the answer.
    pub turns: Vec<ToolTurn>,
}

/// One recorded tool interaction within a completion's transcript.
#[derive(Clone, Debug)]
pub struct ToolTurn {
    /// The tool the model called.
    pub tool: String,
    /// The arguments the model supplied.
    pub args: serde_json::Value,
    /// The result the host returned.
    pub result: serde_json::Value,
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
                 conforming to this JSON Schema, and nothing else:\n{schema}",
                schema = spec.schema
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

    /// The candidate answer in a model's final text: the text itself for
    /// `text`; for `json` and `schema`, the whole text when it parses as
    /// JSON, otherwise the largest fenced or bracketed JSON value, and the
    /// raw text when there is none. A courtesy for providers that wrap JSON
    /// in prose, never a gate — the guest's check decides.
    #[must_use]
    pub fn candidate(&self, text: &str) -> String {
        match self {
            Self::Text => text.to_owned(),
            Self::Json | Self::Schema(_) => extract_json(text)
                .iter()
                .map(ToString::to_string)
                .max_by_key(String::len)
                .unwrap_or_else(|| text.to_owned()),
        }
    }
}

// Every JSON value in `text`: the whole text alone when it parses, else the
// bodies of "```" fences and every bracketed block, each parsed whole. A
// block that does not parse — prose in brackets, code, a document cut short
// — is passed over entire, never read for the values inside it, so a
// fragment of a broken answer is never the answer.
fn extract_json(text: &str) -> Vec<Value> {
    // the whole text is one value
    let text = text.trim();
    if let Ok(value) = serde_json::from_str(text) {
        return vec![value];
    }

    // fence bodies are the odd-indexed chunks between the delimiters, minus
    // their language-tag line
    let mut values = Vec::new();
    for body in text.split("```").skip(1).step_by(2) {
        let body = body.split_once('\n').map_or(body, |(_tag, body)| body);
        if let Ok(value) = serde_json::from_str(body.trim()) {
            values.push(value);
        }
    }

    // `{` / `[` blocks
    let mut rest = text;
    while let Some(start) = rest.find(['{', '[']) {
        let end = start + block_len(&rest[start..]);
        if let Ok(value) = serde_json::from_str(&rest[start..end]) {
            values.push(value);
        }
        rest = &rest[end..];
    }

    values
}

// The length of the bracketed block opening `text`: through the close that
// brings its depth back to zero, brackets inside strings not counted, or all
// of `text` when none does.
fn block_len(text: &str) -> usize {
    let (mut depth, mut quoted, mut escaped) = (0_usize, false, false);
    for (at, c) in text.char_indices() {
        match c {
            _ if escaped => escaped = false,
            '\\' if quoted => escaped = true,
            '"' => quoted = !quoted,
            _ if quoted => {}
            '{' | '[' => depth += 1,
            '}' | ']' => {
                depth -= 1;
                if depth == 0 {
                    return at + c.len_utf8();
                }
            }
            _ => {}
        }
    }

    text.len()
}

// `candidate` and `instruction` are pure; backends drive them directly
#[cfg(test)]
mod tests {
    use super::Format;
    use crate::host::generated::omnia::model::completion::Schema;

    const PASS: &str = r#"{"verdict":"pass"}"#;

    fn verdict_schema() -> Format {
        Format::Schema(Schema {
            name: "verdict".to_owned(),
            schema: r#"{"type":"object"}"#.to_owned(),
        })
    }

    fn candidate(text: &str) -> String {
        verdict_schema().candidate(text)
    }

    #[test]
    fn text_passthrough() {
        assert_eq!(Format::Text.candidate("  plain {not: json}  "), "  plain {not: json}  ");
    }

    #[test]
    fn json_document() {
        assert_eq!(Format::Json.candidate(PASS), PASS);
    }

    #[test]
    fn fenced_json() {
        assert_eq!(candidate("```json\n{\"verdict\":\"pass\"}\n```"), PASS);
    }

    // a scalar answer has no bracket to open it, so the fence is its only way in
    #[test]
    fn fenced_scalar() {
        assert_eq!(candidate("```json\n\"pass\"\n```"), r#""pass""#);
    }

    #[test]
    fn json_with_preamble() {
        assert_eq!(candidate("Done.\n{\"verdict\":\"pass\"}\n"), PASS);
    }

    // the largest value is the answer: a worked example before it, a
    // fragment of it repeated after it, and a citation beside it are smaller
    #[test]
    fn largest_value_wins() {
        assert_eq!(
            candidate(
                "For example {\"verdict\":\"fail\"} — and mine:\n{\"findings\":[],\"verdict\":\"pass\"}"
            ),
            r#"{"findings":[],"verdict":"pass"}"#
        );
        let document = r#"{"requirements":[{"scenarios":[],"subject":"a.run"}]}"#;
        assert_eq!(
            candidate(&format!("{document}\n{{\"scenarios\":[],\"subject\":\"a.run\"}}")),
            document
        );
        assert_eq!(candidate("{\"verdict\":\"pass\"}\nSee [1] and [2]."), PASS);
    }

    // a bracketed block that does not parse whole is passed over, not read
    // for the values inside it — prose in brackets, code in a fence, a key
    // named with no `:` — and the value beside it is read
    #[test]
    fn blocks_that_are_not_json() {
        for text in [
            "[thinking] weighing the claims.\n{\"verdict\":\"pass\"}",
            "{thinking} done.\n```json\n{\"verdict\":\"pass\"}\n```",
            "Done [thinking] with {care}; see [[note]] and {{name}}.\n{\"verdict\":\"pass\"}",
            "Scored [1 point] as a [true story] — [3 items, mostly]:\n{\"verdict\":\"pass\"}",
            "```python\nxs = [x for x in xs]\n```\n{\"verdict\":\"pass\"}",
            "```js\nconst reply = { \"verdict\": verdict };\n```\n{\"verdict\":\"pass\"}",
            "{\"verdict\":\"pass\"}\nThe {\"verdict\"} key is required.",
        ] {
            assert_eq!(candidate(text), PASS, "{text}");
        }
    }

    // a bracket or an escaped quote inside a string does not end the block
    #[test]
    fn brackets_in_strings() {
        let answer = r#"{"finding":"an unmatched } and \" quote","verdict":"pass"}"#;
        assert_eq!(candidate(&format!("Note:\n{answer}")), answer);
    }

    // a document cut short, or broken mid-way, is handed back whole — bare,
    // fenced, or after prose — never a well-formed member of it
    #[test]
    fn malformed_document() {
        for text in [
            "{\n  \"requirements\": [{\"scenarios\":[],\"subject\":\"a.run\"}],\n",
            "```json\n{\n  \"requirements\": [{\"scenarios\":[],\"subject\":\"a.run\"}],\n```",
            "Here is my answer: {\"findings\": [], \"verdict\": \"pass\"",
            "{\"verdict\": \"pass\", // reason\n  \"findings\": [{\"claim\":\"a\"}]}",
        ] {
            assert_eq!(candidate(text), text);
        }
    }

    #[test]
    fn no_json() {
        assert_eq!(candidate("not json"), "not json");
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
