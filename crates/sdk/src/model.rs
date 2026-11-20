//! `omnia:model`, prompt completions on the models a deployment declares.
//!
//! [`Request::complete`] runs one completion on the model a [`Request`]
//! names, [`DEFAULT`] unless it names another. [`Request::complete_with`]
//! answers the model's tool calls on the way, and
//! [`Request::complete_checked`] also judges each answer the model proposes.
//! [`open`] opens a model for the raw [`completion`] session.
//!
//! A [`Request`] holds the `omnia:model` records, but for the
//! `grants.workspace` lend, a `wasi:filesystem` descriptor. A guest names the
//! directory with the plain [`Request::workspace`] path, which the completion
//! resolves against the guest's preopens by the lend rule every lending
//! capability shares. The longest preopen whose name prefixes the path is the
//! lent root descriptor, and the remainder is the location's subpath.
//!
//! A completion that spends a budget before it has an answer is the inner
//! `Err`, an [`Exhausted`]. The typed [`Question`] judges each candidate as a
//! `JsonSchema + Deserialize` answer type.

pub mod prompt;
mod question;

use std::fmt;
use std::future::{Future, IntoFuture, Ready};

use schemars::JsonSchema;
use serde::de::DeserializeOwned;
use wasip3::filesystem::preopens;

pub use self::question::{Findings, Question, ToolFuture, Tools};
pub use crate::bindings::imports::omnia::model::completion::Model;
pub use crate::bindings::imports::omnia::model::types::{
    Budget, Candidate, Effort, Exhausted, Format, Generation, Message, Reply, Role, Schema, Tool,
    ToolCall, Usage, WorkerGrant,
};
pub use crate::bindings::imports::omnia::model::{completion, types};
use crate::lend::resolve_lend;
use crate::{Result, wit_stream};

/// The model a [`Request`] opens when it names none.
pub const DEFAULT: &str = "default";

/// Open the model `name`, a binding the deployment declares.
///
/// A [`Request`] opens its model itself, so a guest opens one only to run
/// the raw [`completion`] session.
///
/// # Errors
///
/// Returns an [`Error`](crate::Error) when the deployment does not declare
/// `name`, or the host fails the call.
pub async fn open(name: &str) -> Result<Model> {
    Ok(completion::open(name.to_owned()).await?)
}

/// What the guest asks of a model in one completion.
#[derive(Clone, Debug, Default, bon::Builder)]
pub struct Request {
    /// The model to open, a binding the deployment declares. `None` opens
    /// [`DEFAULT`].
    #[builder(into)]
    pub model: Option<String>,
    /// Standing instructions, read before `messages`.
    #[builder(into)]
    pub system: Option<String>,
    /// The conversation so far, oldest first. Messages that hold no text are
    /// `invalid`.
    pub messages: Vec<Message>,
    /// The controls on how the model generates. `None` takes the backend's
    /// defaults.
    pub generation: Option<Generation>,
    /// The shape the answer should take, which steers the model and
    /// guarantees nothing.
    #[builder(default)]
    pub format: Format,
    /// The tools the guest runs for the model.
    ///
    /// `read`, `list`, and `write` are the host's, and a name given twice is
    /// `invalid`.
    #[builder(default)]
    pub tools: Vec<Tool>,
    /// The deployment-local path of the directory the completion works in,
    /// `None` to lend none.
    ///
    /// The path is a preopen or beneath one: `"."` lends the project mount,
    /// and `"/mount/sub"` a directory beneath `/mount`.
    #[builder(into)]
    pub workspace: Option<String>,
    /// What the backend's worker may reach beyond the
    /// [`workspace`](Self::workspace), `None` to grant nothing more.
    pub worker: Option<WorkerGrant>,
}

impl Request {
    /// Run the completion and return its reply.
    ///
    /// Each tool call the model makes fails back to it.
    ///
    /// # Errors
    ///
    /// Returns an [`Error`](crate::Error) as
    /// [`complete_with`](Self::complete_with) does.
    pub async fn complete(self) -> Result<Result<Reply, Exhausted>> {
        self.complete_with(|call: ToolCall| async move {
            Err::<String, String>(format!(
                "tool `{}` has no handler: answer tool calls with complete_with",
                call.name
            ))
        })
        .await
    }

    /// Run the completion, answering each of the model's tool calls with
    /// `tools`, and return its reply.
    ///
    /// The calls are answered one at a time. An `Err` is a failure the model
    /// reads, and it may call again.
    ///
    /// # Errors
    ///
    /// Returns an [`Error`](crate::Error) when:
    ///
    /// - the request sets a [`worker`](Self::worker) without a
    ///   [`workspace`](Self::workspace), as `invalid`;
    /// - the [`workspace`](Self::workspace) matches no preopen, as `denied`;
    /// - the host rejects the request or fails the completion.
    ///
    /// An [`Exhausted`] is the inner `Err`: the completion spent a budget
    /// before it had an answer.
    pub async fn complete_with<T, F>(self, tools: T) -> Result<Result<Reply, Exhausted>>
    where
        T: FnMut(ToolCall) -> F,
        F: Future<Output = Result<String, String>>,
    {
        self.run(tools, None::<fn(Candidate) -> Ready<Result<(), String>>>).await
    }

    /// Run the completion, answering each tool call with `tools` and judging
    /// each candidate answer with `check`, and return the reply.
    ///
    /// The reply is the first candidate `check` accepts. An `Err` is the
    /// correction the model reads before it proposes another.
    ///
    /// # Errors
    ///
    /// Returns an [`Error`](crate::Error) as
    /// [`complete_with`](Self::complete_with) does.
    pub async fn complete_checked<T, F, C, G>(
        self, tools: T, check: C,
    ) -> Result<Result<Reply, Exhausted>>
    where
        T: FnMut(ToolCall) -> F,
        F: Future<Output = Result<String, String>>,
        C: FnMut(Candidate) -> G,
        G: Future<Output = Result<(), String>>,
    {
        self.run(tools, Some(check)).await
    }

    async fn run<T, F, C, G>(
        self, mut tools: T, mut check: Option<C>,
    ) -> Result<Result<Reply, Exhausted>>
    where
        T: FnMut(ToolCall) -> F,
        F: Future<Output = Result<String, String>>,
        C: FnMut(Candidate) -> G,
        G: Future<Output = Result<(), String>>,
    {
        // a worker grant nests in the lend it reaches beyond
        if self.worker.is_some() && self.workspace.is_none() {
            let refusal = "a worker grant requires a workspace lend".to_owned();
            return Err(types::Error::Invalid(refusal).into());
        }

        // the lent workspace borrows a descriptor, so the table outlives the session
        let directories =
            if self.workspace.is_some() { preopens::get_directories() } else { vec![] };
        let location = match self.workspace.as_deref() {
            None => None,
            Some(path) => {
                let Some((root, subpath)) = resolve_lend(&directories, path) else {
                    let refusal = format!("workspace lend `{path}` matches no preopen");
                    return Err(types::Error::Denied(refusal).into());
                };
                Some(types::Location {
                    root,
                    subpath: subpath.to_owned(),
                })
            }
        };

        let wire = completion::Request {
            system: self.system,
            messages: self.messages,
            generation: self.generation,
            format: self.format,
            tools: self.tools,
            grants: types::Grants {
                workspace: location.map(|location| types::WorkspaceGrant {
                    location,
                    worker: self.worker,
                }),
            },
            check: check.is_some(),
        };

        // begin the completion on the model the request names
        let model = open(self.model.as_deref().unwrap_or(DEFAULT)).await?;
        let (mut results, results_rx) = wit_stream::new();
        let completion::Session { mut calls, reply } = model.complete(wire, results_rx).await?;

        // serial by design: each result feeds the same model turn; a rejected
        // write means the host stopped reading, and the closed stream ends the loop
        let calls_loop = async {
            while let Some(call) = calls.next().await {
                let result = match call {
                    types::Call::Tool(call) => {
                        let id = call.id.clone();
                        let output = tools(call).await;
                        types::CallResult::Tool(types::ToolResult { id, output })
                    }
                    types::Call::Check(candidate) => {
                        let id = candidate.id.clone();
                        let outcome = match check.as_mut() {
                            Some(check) => check(candidate).await,
                            None => Err("this completion asked for no check".to_owned()),
                        };
                        types::CallResult::Check(types::Verdict { id, outcome })
                    }
                };
                let _ = results.write_one(result).await;
            }
        };

        // the host always resolves the reply, so the join cannot hang
        let ((), outcome) = futures::join!(calls_loop, reply.into_future());
        Ok(outcome?)
    }
}

impl Message {
    /// A [`User`](Role::User) message holding `content`.
    #[must_use]
    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: Role::User,
            content: content.into(),
        }
    }

    /// An [`Assistant`](Role::Assistant) message holding `content`.
    #[must_use]
    pub fn assistant(content: impl Into<String>) -> Self {
        Self {
            role: Role::Assistant,
            content: content.into(),
        }
    }

    /// A [`System`](Role::System) message holding `content`.
    #[must_use]
    pub fn system(content: impl Into<String>) -> Self {
        Self {
            role: Role::System,
            content: content.into(),
        }
    }
}

#[expect(clippy::derivable_impls, reason = "a generated type takes no derive")]
impl Default for Generation {
    fn default() -> Self {
        Self {
            temperature: None,
            top_p: None,
            max_tokens: None,
            stop_sequences: Vec::new(),
            seed: None,
            effort: None,
        }
    }
}

impl Format {
    /// `T`'s JSON Schema (draft 2020-12) under `name`.
    #[must_use]
    pub fn schema_of<T: JsonSchema>(name: &str) -> Self {
        Self::Schema(Schema {
            name: name.to_owned(),
            schema: schema_of::<T>(),
        })
    }
}

#[expect(clippy::derivable_impls, reason = "a generated type takes no derive")]
impl Default for Format {
    fn default() -> Self {
        Self::Text
    }
}

fn schema_of<T: JsonSchema>() -> String {
    schemars::schema_for!(T).to_value().to_string()
}

impl Tool {
    /// A tool named `name` whose arguments are `T`'s JSON Schema.
    #[must_use]
    pub fn of<T: JsonSchema>(name: &str, description: &str) -> Self {
        Self {
            name: name.to_owned(),
            description: Some(description.to_owned()),
            input_schema: schema_of::<T>(),
        }
    }
}

#[expect(clippy::derivable_impls, reason = "a generated type takes no derive")]
impl Default for WorkerGrant {
    fn default() -> Self {
        Self {
            egress: Vec::new(),
            read: Vec::new(),
            env: Vec::new(),
        }
    }
}

impl ToolCall {
    /// The call's arguments, deserialized into `T`.
    ///
    /// # Errors
    ///
    /// Returns failure text for the model when the arguments do not match
    /// `T`.
    pub fn arguments<T: DeserializeOwned>(&self) -> Result<T, String> {
        serde_json::from_str(&self.arguments).map_err(|error| format!("invalid arguments: {error}"))
    }
}

impl fmt::Display for Budget {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Responses => "responses",
            Self::Tokens => "tokens",
            Self::ToolCalls => "tool-calls",
            Self::CallTime => "call-time",
            Self::CompletionTime => "completion-time",
        })
    }
}

impl fmt::Display for Exhausted {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "the {} budget is exhausted: {}", self.budget, self.detail)
    }
}

impl std::error::Error for Exhausted {}
