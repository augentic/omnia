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

impl std::fmt::Display for Format {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Text => "text",
            Self::Json => "json",
            Self::Schema(_) => "schema",
        })
    }
}

// Every JSON value in `text`: the whole text alone when it parses, else each
// span that parses whole. A span that does not — prose in brackets, code in
// a fence, a document cut short — is passed over entire, never read for the
// values inside it, so a fragment of a broken answer is never the answer.
fn extract_json(text: &str) -> Vec<Value> {
    // the whole text is one value
    let text = text.trim();
    if let Ok(value) = serde_json::from_str(text) {
        return vec![value];
    }

    Spans { rest: text }.filter_map(|span| serde_json::from_str(span).ok()).collect()
}

// Fence bodies and bracketed blocks, left to right; whichever opens first
// owns what it spans. A fence delimiter is a "```" that begins a line, as
// Markdown places it: one inside a line — in prose, or in a JSON string,
// which cannot hold a raw newline — delimits nothing.
struct Spans<'a> {
    rest: &'a str,
}

impl<'a> Spans<'a> {
    // The offset of the next fence delimiter in `rest`.
    fn fence_at(&self) -> Option<usize> {
        if self.rest.starts_with("```") {
            return Some(0);
        }
        self.rest.find("\n```").map(|at| at + 1)
    }

    // The body of the fence at the head of `rest`, minus its language-tag
    // line; `rest` moves past the closing delimiter, or to the end when none
    // closes it.
    fn fence(&mut self) -> &'a str {
        let inner = &self.rest[3..];
        let (body, rest) = inner.split_once("\n```").unwrap_or((inner, ""));
        self.rest = rest;
        body.split_once('\n').map_or(body, |(_tag, body)| body).trim()
    }

    // The bracketed block at the head of `rest`: through the close that
    // brings its depth back to zero, brackets inside strings not counted.
    fn block(&mut self) -> &'a str {
        let (mut depth, mut quoted, mut escaped) = (0_usize, false, false);
        let closed = self.rest.char_indices().find_map(|(at, c)| {
            match c {
                _ if escaped => escaped = false,
                '\\' if quoted => escaped = true,
                '"' => quoted = !quoted,
                _ if quoted => {}
                '{' | '[' => depth += 1,
                '}' | ']' => {
                    depth -= 1;
                    if depth == 0 {
                        return Some(at + c.len_utf8());
                    }
                }
                _ => {}
            }
            None
        });

        // one that never closes is abandoned at the next fence delimiter
        let end = closed.or_else(|| self.fence_at()).unwrap_or(self.rest.len());
        let (body, rest) = self.rest.split_at(end);
        self.rest = rest;
        body
    }
}

impl<'a> Iterator for Spans<'a> {
    type Item = &'a str;

    fn next(&mut self) -> Option<&'a str> {
        let start = [self.fence_at(), self.rest.find(['{', '['])].into_iter().flatten().min()?;
        self.rest = &self.rest[start..];
        Some(if self.rest.starts_with("```") { self.fence() } else { self.block() })
    }
}
