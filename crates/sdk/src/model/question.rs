//! `Question<T>`: a completion whose answer is one `T`.
//!
//! The request's `format` is `T`'s JSON Schema, a steering hint for the
//! provider, and the question judges each candidate itself. A candidate is
//! deserialized into `T` and handed to the caller's closure. A miss becomes
//! the correction turn (one default template, replaceable per question), and
//! a hit is captured and returned. The reply text is never re-parsed.

use std::fmt;
use std::future::{Future, ready};
use std::marker::PhantomData;
use std::pin::Pin;
use std::sync::Arc;

use schemars::JsonSchema;
use serde::de::DeserializeOwned;
use serde_json::Value;

use super::{Candidate, Exhausted, Format, Message, Request, Tool, ToolCall, WorkerGrant, types};
use crate::Result;

/// What a check found wrong with a candidate, one finding per line of the
/// correction turn.
pub type Findings = Vec<String>;

/// The future a tool handler returns.
pub type ToolFuture = Pin<Box<dyn Future<Output = Result<String, String>> + Send + 'static>>;

/// The caller's handler for the request's tools.
pub type Tools = Box<dyn FnMut(ToolCall) -> ToolFuture + Send>;

/// Renders the correction turn for a rejected candidate from the previous
/// answer and the findings against it.
pub type Correction = Arc<dyn Fn(&str, &[String]) -> String + Send + Sync>;

/// A question whose answer is one `T`.
pub struct Question<T> {
    request: Request,
    correction: Correction,
    answer: PhantomData<fn() -> T>,
}

impl<T: JsonSchema + DeserializeOwned + Send> Question<T> {
    /// A question steered by `T`'s schema under `name`.
    #[must_use]
    pub fn new(name: &str) -> Self {
        Self {
            request: Request {
                format: Format::schema_of::<T>(name),
                ..Request::default()
            },
            correction: Arc::new(default_correction),
            answer: PhantomData,
        }
    }

    /// The standing instructions, read before the messages.
    #[must_use]
    pub fn system(mut self, prose: impl Into<String>) -> Self {
        self.request.system = Some(prose.into());
        self
    }

    /// The model to open; see [`Request::model`].
    #[must_use]
    pub fn model(mut self, model: impl Into<String>) -> Self {
        self.request.model = Some(model.into());
        self
    }

    /// The messages preceding the one [`Self::ask`] adds.
    #[must_use]
    pub fn messages(mut self, turns: Vec<Message>) -> Self {
        self.request.messages = turns;
        self
    }

    /// The tools the guest runs for the model.
    #[must_use]
    pub fn tools(mut self, tools: Vec<Tool>) -> Self {
        self.request.tools = tools;
        self
    }

    /// The directory the completion works in; see [`Request::workspace`].
    #[must_use]
    pub fn workspace(mut self, lend: impl Into<String>) -> Self {
        self.request.workspace = Some(lend.into());
        self
    }

    /// What the backend's worker may reach beyond the lend; see
    /// [`Request::worker`].
    #[must_use]
    pub fn worker(mut self, grant: impl Into<WorkerGrant>) -> Self {
        self.request.worker = Some(grant.into());
        self
    }

    /// Post-process the steering schema, such as subject enums, exact counts,
    /// or id patterns. Hints for the provider, never the gate.
    #[must_use]
    pub fn schema(mut self, edit: impl FnOnce(&mut Value)) -> Self {
        if let Format::Schema(spec) = &mut self.request.format
            && let Ok(mut schema) = serde_json::from_str::<Value>(&spec.schema)
        {
            edit(&mut schema);
            spec.schema = schema.to_string();
        }
        self
    }

    /// Replace the correction turn's template.
    ///
    /// `render(previous, findings)` is the user message a rejected candidate
    /// earns, `previous` being the candidate text and `findings` what the
    /// check held against it, or the one deserialization failure. The default
    /// is a `## Previous answer (rejected)` / `## Findings` template.
    #[must_use]
    pub fn correction(
        mut self, render: impl Fn(&str, &[String]) -> String + Send + Sync + 'static,
    ) -> Self {
        self.correction = Arc::new(render);
        self
    }

    /// The request as built so far.
    #[must_use]
    pub const fn request(&self) -> &Request {
        &self.request
    }

    /// Run one completion with `turn` appended as the user message, and
    /// return the first candidate `check` accepts.
    ///
    /// Each candidate the backend proposes is deserialized into `T` and
    /// judged by `check`. A deserialization failure or `Err(findings)` is the
    /// correction turn, and `Ok` accepts the candidate.
    ///
    /// # Errors
    ///
    /// Returns an [`Error`](crate::Error) when:
    ///
    /// - a candidate failed to deserialize and no other was accepted, as
    ///   `invalid`, since the schema and `T` disagree;
    /// - the backend finished with no accepted answer, as `other`;
    /// - the completion fails as [`Request::complete_with`] does.
    ///
    /// An [`Exhausted`] is the inner `Err`: the backend spent a budget on
    /// rejected candidates.
    pub async fn ask(
        &self, turn: impl Into<String>, tools: Option<Tools>,
        mut check: impl FnMut(&T) -> Result<(), Findings> + Send,
    ) -> Result<Result<T, Exhausted>> {
        let mut request = self.request.clone();
        request.messages.push(Message::user(turn));

        let mut tools = tools;
        let tool_handler = |call: ToolCall| -> ToolFuture {
            match tools.as_mut() {
                Some(tools) => tools(call),
                None => Box::pin(ready(Err(format!(
                    "tool `{}` has no handler: pass one to Question::ask",
                    call.name
                )))),
            }
        };

        let mut accepted: Option<T> = None;
        let mut mismatch: Option<String> = None;
        let mut candidates = 0_usize;
        let correction = &self.correction;
        let check_handler = |candidate: Candidate| {
            candidates += 1;
            let verdict = match serde_json::from_str::<T>(&candidate.answer) {
                Err(error) => {
                    let finding = format!("the answer does not match the expected shape: {error}");
                    mismatch = Some(finding.clone());
                    Err(correction(&candidate.answer, &[finding]))
                }
                Ok(answer) => match check(&answer) {
                    Ok(()) => {
                        accepted = Some(answer);
                        Ok(())
                    }
                    Err(findings) => Err(correction(&candidate.answer, &findings)),
                },
            };
            ready(verdict)
        };

        let outcome = request.complete_checked(tool_handler, check_handler).await?;
        match (outcome, accepted, mismatch) {
            (Ok(_), Some(answer), _) => Ok(Ok(answer)),
            (Ok(_), None, _) if candidates == 0 => {
                let detail = "the backend finished without running the check".to_owned();
                Err(types::Error::Other(detail).into())
            }
            (Ok(_), None, _) => {
                let detail = "the backend finished on a candidate the check rejected".to_owned();
                Err(types::Error::Other(detail).into())
            }
            // the schema and `T` disagree: a guest bug more rounds would not fix
            (Err(_), None, Some(mismatch)) => {
                let detail = format!("schema and answer type disagree: {mismatch}");
                Err(types::Error::Invalid(detail).into())
            }
            (Err(exhausted), _, _) => Ok(Err(exhausted)),
        }
    }
}

impl<T> Clone for Question<T> {
    fn clone(&self) -> Self {
        Self {
            request: self.request.clone(),
            correction: Arc::clone(&self.correction),
            answer: PhantomData,
        }
    }
}

impl<T> fmt::Debug for Question<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Question").field("request", &self.request).finish_non_exhaustive()
    }
}

// The default correction turn for a rejected candidate.
fn default_correction(previous: &str, findings: &[String]) -> String {
    format!(
        "## Previous answer (rejected)\n\n{previous}\n\n## Findings\n\n{}\n\nProduce a corrected, \
         complete answer that resolves every finding.",
        findings.join("\n")
    )
}
