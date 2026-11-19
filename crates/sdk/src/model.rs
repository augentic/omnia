//! `omnia:model`, prompt completions on the models a deployment declares.
//!
//! [`Request::complete`] runs one completion on the model a [`Request`]
//! names, [`DEFAULT`] unless it names another, and
//! [`Request::complete_with`] answers the model's tool calls on the way. The
//! raw session is [`completion`].
//!
//! The records mirror the `omnia:model` records, except the `grants.workspace`
//! lend, a `wasi:filesystem` descriptor. A guest names the directory with the
//! plain [`Request::workspace`] path, which the completion resolves against
//! the guest's preopens by the lend rule every lending capability shares: the
//! longest preopen whose name prefixes the path becomes the lent root
//! descriptor, and the remainder rides as the location's subpath.
//!
//! Acceptance of an answer is the guest's: a request with [`Request::check`]
//! set receives each candidate as a tool call named `check` and answers
//! `Ok` to finish or `Err(correction)` to send the model round again. The
//! typed [`Question`] runs that exchange for a `JsonSchema + Deserialize`
//! answer type.

pub mod prompt;
mod question;

use std::future::Future;

use schemars::JsonSchema;
use serde::de::DeserializeOwned;
use wasip3::filesystem::preopens;

pub use self::question::{Findings, Question, ToolFuture, Tools};
pub use crate::bindings::imports::omnia::model::{completion, types};
use crate::lend::resolve_lend;
use crate::wit_stream;

/// The model a [`Request`] opens when it names none.
pub const DEFAULT: &str = "default";

/// The reserved tool name a `check` candidate arrives under.
pub const CHECK_TOOL: &str = "check";

/// Complete request for one completion.
#[derive(Clone, Debug, Default, PartialEq, Eq, bon::Builder)]
pub struct Request {
    /// The model to open, a binding the deployment declares. `None` opens
    /// `default`.
    #[builder(into)]
    pub model: Option<String>,
    /// System / instructions channel.
    #[builder(into)]
    pub system: Option<String>,
    /// Chat turns sent to the provider. Must not be empty.
    pub messages: Vec<Message>,
    /// Sampling and length controls.
    pub generation: Option<Generation>,
    /// Output shape hint steering the provider; nothing is validated
    /// against it.
    #[builder(default)]
    pub format: Format,
    /// Guest-declared functions, offered beside the host-injected tools. The
    /// names `read`, `list`, `write`, and `check` are reserved, and a name
    /// given twice is refused.
    #[builder(default)]
    pub tools: Vec<Tool>,
    /// Ask the guest to accept each candidate answer: the candidate arrives
    /// at the [`Request::complete_with`] handler as a [`ToolCall`] named
    /// `check` whose `arguments` are the candidate text. `Ok` accepts it as
    /// the reply; `Err(text)` is appended to the conversation verbatim as
    /// the correction turn and the backend goes round again.
    #[builder(default)]
    pub check: bool,
    /// Deployment-local path of the directory to lend through
    /// `grants.workspace`, giving the backend (and any spawned agent) that
    /// directory. The path must sit on (or beneath) a preopen: `"."` lends
    /// the shared project mount, `"/mount/sub"` lends a subdirectory of
    /// `/mount`. Test doubles observe the raw string. `None` lends nothing.
    #[builder(into)]
    pub workspace: Option<String>,
    /// What the worker a backend runs for this completion may reach beyond
    /// its lend. Requires [`Request::workspace`]: a worker reaches beyond a
    /// lend it has. `None` grants nothing beyond the lend.
    pub worker: Option<WorkerGrant>,
}

impl Request {
    /// Run the completion and return its one reply.
    ///
    /// Any tool call the model issues fails back to it. A request that
    /// declares tools, or sets [`check`](Self::check), answers them through
    /// [`complete_with`](Self::complete_with) instead.
    ///
    /// # Errors
    ///
    /// The [`Error`] that names how the completion failed.
    pub async fn complete(self) -> Result<Reply, Error> {
        self.complete_with(|call: ToolCall| async move {
            Err::<String, String>(format!(
                "tool `{}` has no handler: answer tool calls with complete_with",
                call.name
            ))
        })
        .await
    }

    /// Run the completion, answering each of the model's tool calls with
    /// `handler`.
    ///
    /// The calls arrive as [`ToolCall`] values, answered serially, and each
    /// result feeds the same model turn. Results correlate by id, so the raw
    /// [`completion`] session handles calls in parallel.
    ///
    /// # Errors
    ///
    /// - [`Error::InvalidRequest`] for a worker grant without a workspace, a
    ///   workspace that matches no preopen, or a tool named
    ///   [`CHECK_TOOL`].
    /// - [`Error::BudgetExhausted`] when the model ran out of rounds, with
    ///   the last correction when a `check` rejected the final candidate.
    /// - The [`Error`] the host reports, under the variant of its name.
    pub async fn complete_with<H, F>(self, mut handler: H) -> Result<Reply, Error>
    where
        H: FnMut(ToolCall) -> F,
        F: Future<Output = Result<String, String>>,
    {
        // a worker grant nests in the lend it reaches beyond
        if self.worker.is_some() && self.workspace.is_none() {
            return Err(Error::InvalidRequest(
                "a worker grant requires a workspace lend".to_owned(),
            ));
        }

        // the lent workspace borrows a descriptor, so the table outlives the session
        let directories =
            if self.workspace.is_some() { preopens::get_directories() } else { vec![] };
        let workspace = match self.workspace.as_deref() {
            None => None,
            Some(path) => match resolve_lend(&directories, path) {
                Some((root, subpath)) => Some(types::Location {
                    root,
                    subpath: subpath.to_string(),
                }),
                None => {
                    return Err(Error::InvalidRequest(format!(
                        "workspace lend `{path}` matches no preopen"
                    )));
                }
            },
        };

        // the guest's tools, refusing the name a check arrives under
        let mut tools = Vec::new();
        for Tool::Function(function) in self.tools {
            if function.name == CHECK_TOOL {
                return Err(Error::InvalidRequest(format!("reserved tool name: {CHECK_TOOL}")));
            }
            tools.push(function.into());
        }

        let wire = completion::Request {
            system: self.system,
            messages: self.messages.into_iter().map(Into::into).collect(),
            generation: self.generation.map(Into::into),
            format: self.format.into(),
            tools,
            grants: types::Grants {
                workspace: workspace.map(|location| types::WorkspaceGrant {
                    location,
                    worker: self.worker.map(Into::into),
                }),
            },
            check: self.check,
        };

        // begin the completion on the model the request names
        let name = self.model.unwrap_or_else(|| DEFAULT.to_owned());
        let model = completion::open(name).await?;
        let (mut results, results_rx) = wit_stream::new();
        let completion::Session { mut calls, reply } = model.complete(wire, results_rx).await?;

        // serial by design: each result feeds the same model turn; a rejected
        // write means the host stopped reading, and the closed stream ends the loop
        let mut correction = None;
        let calls_loop = async {
            while let Some(call) = calls.next().await {
                let result = match call {
                    types::Call::Tool(call) => {
                        correction = None;
                        let id = call.id.clone();
                        let output = handler(ToolCall {
                            id: call.id,
                            name: call.name,
                            arguments: call.arguments,
                        })
                        .await;
                        types::CallResult::Tool(types::ToolResult { id, output })
                    }
                    types::Call::Check(candidate) => {
                        let id = candidate.id.clone();
                        let outcome = handler(ToolCall {
                            id: candidate.id,
                            name: CHECK_TOOL.to_owned(),
                            arguments: candidate.answer,
                        })
                        .await
                        .map(drop);
                        correction = outcome.clone().err();
                        types::CallResult::Check(types::Verdict { id, outcome })
                    }
                };
                let _ = results.write_one(result).await;
            }
        };

        // the host always resolves the reply, so the join cannot hang
        let ((), outcome) = futures::join!(calls_loop, std::future::IntoFuture::into_future(reply));
        match outcome? {
            Ok(reply) => Ok(reply.into()),
            Err(exhausted) => Err(Error::BudgetExhausted(correction.unwrap_or(exhausted.detail))),
        }
    }
}

/// What a backend's worker may reach beyond its lend for one completion.
///
/// Spelled by the guest that knows the toolchain the turn runs, never by
/// the deployment. The host holds each entry to its shape and the backend
/// lays what exists on its machine.
#[derive(Clone, Debug, Default, PartialEq, Eq, bon::Builder)]
pub struct WorkerGrant {
    /// Host names the worker's commands may reach through the backend's
    /// egress proxy: a name whole, or `*.suffix` for a domain and every
    /// host beneath it. Never an IP literal, a port, a scheme, or a path.
    #[builder(default)]
    pub egress: Vec<String>,
    /// Directories the worker may read beside its lend and the system, each
    /// `~/<path>` beneath the operator's home. `<path>` is a plain relative
    /// path: `/`-separated, no segment empty, `.` or `..`, no `\`, so never
    /// the home itself. One the machine lacks grants nothing.
    #[builder(default)]
    pub read: Vec<String>,
    /// Environment variables set for the worker, each a name of the shape
    /// `[A-Za-z_][A-Za-z0-9_]*` and a value whose leading `~/` the backend
    /// expands to the operator's home.
    #[builder(default)]
    pub env: Vec<(String, String)>,
}

/// One chat turn passed to the provider API.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Message {
    /// Turn author.
    pub role: Role,
    /// Turn body text.
    pub content: String,
}

impl Message {
    /// A [`User`](Role::User) turn carrying `content`.
    #[must_use]
    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: Role::User,
            content: content.into(),
        }
    }

    /// An [`Assistant`](Role::Assistant) turn carrying `content`.
    #[must_use]
    pub fn assistant(content: impl Into<String>) -> Self {
        Self {
            role: Role::Assistant,
            content: content.into(),
        }
    }

    /// A [`System`](Role::System) turn carrying `content`.
    #[must_use]
    pub fn system(content: impl Into<String>) -> Self {
        Self {
            role: Role::System,
            content: content.into(),
        }
    }
}

/// Chat turn author.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    /// System / instructions channel.
    System,
    /// End-user turn.
    User,
    /// Model turn.
    Assistant,
}

/// Sampling and length controls. Omitted fields defer to backend defaults.
#[derive(Clone, Debug, Default, PartialEq, bon::Builder)]
pub struct Generation {
    /// Sampling temperature.
    pub temperature: Option<f32>,
    /// Nucleus sampling threshold.
    pub top_p: Option<f32>,
    /// Maximum output tokens.
    pub max_tokens: Option<u32>,
    /// Sequences that halt generation.
    #[builder(default)]
    pub stop: Vec<String>,
    /// Seed for reproducible sampling when the provider supports it.
    pub seed: Option<u64>,
    /// Reasoning-effort hint for thinking-capable models.
    pub effort: Option<Effort>,
}

// The float fields are sampling controls set from configuration values; NaN
// is never a meaningful setting, so total equality holds.
impl Eq for Generation {}

/// Reasoning-effort hint for models that expose a thinking budget.
///
/// [`Effort::Low`], [`Effort::Medium`], and [`Effort::High`] are portable. A
/// backend fails a level it does not know with [`Error::InvalidRequest`], and
/// one with no reasoning control fails any level with [`Error::Unsupported`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Effort {
    /// Reduced reasoning.
    Low,
    /// Balanced reasoning.
    Medium,
    /// Most reasoning; highest latency and cost.
    High,
    /// A level by the backend's own name, such as `minimal`.
    Other(String),
}

impl Effort {
    /// The level's name, as the backend receives it.
    #[must_use]
    pub const fn as_str(&self) -> &str {
        match self {
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
            Self::Other(level) => level.as_str(),
        }
    }
}

/// Output shape hint steering the provider.
///
/// Passed as `response_format` where the provider constrains decoding,
/// instruction prose where it does not. Nothing is validated against it;
/// acceptance is the request's `check`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Format {
    /// Plain text.
    #[default]
    Text,
    /// Steer toward a JSON object.
    Json,
    /// Steer toward the given JSON Schema.
    Schema(SchemaFormat),
}

/// JSON Schema steering output.
#[derive(Clone, Debug, PartialEq, Eq, bon::Builder)]
pub struct SchemaFormat {
    /// Schema name passed to the provider (e.g. `review_result`).
    #[builder(into)]
    pub name: String,
    /// JSON Schema document the answer should conform to.
    #[builder(into)]
    pub schema: String,
}

impl SchemaFormat {
    /// `T`'s JSON Schema (draft 2020-12) under `name`.
    #[must_use]
    pub fn of<T: JsonSchema>(name: &str) -> Self {
        Self {
            name: name.to_owned(),
            schema: schema_of::<T>(),
        }
    }
}

fn schema_of<T: JsonSchema>() -> String {
    schemars::schema_for!(T).to_value().to_string()
}

/// A tool offered to the model.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Tool {
    /// Guest-declared function tool.
    Function(Function),
}

/// Guest-declared function tool advertised to the model.
#[derive(Clone, Debug, PartialEq, Eq, bon::Builder)]
pub struct Function {
    /// Tool name. Must not collide with the reserved names (`read`, `list`,
    /// `write`, `check`).
    #[builder(into)]
    pub name: String,
    /// Natural-language description for the model.
    #[builder(into)]
    pub description: String,
    /// JSON Schema for the tool's arguments object.
    #[builder(into)]
    pub parameters: String,
}

impl Function {
    /// A function tool whose parameters are `T`'s JSON Schema.
    #[must_use]
    pub fn of<T: JsonSchema>(name: &str, description: &str) -> Self {
        Self {
            name: name.to_owned(),
            description: description.to_owned(),
            parameters: schema_of::<T>(),
        }
    }
}

/// One completion result.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Reply {
    /// The text the request's `check` accepted — or, without a check, the
    /// model's final text as the backend received it.
    pub answer: String,
    /// Token accounting, when the backend reports it.
    pub usage: Option<Usage>,
}

/// Token accounting for one completion, counted over every response the model
/// wrote.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Usage {
    /// Tokens the model read, cached ones included.
    pub input_tokens: u64,
    /// Tokens the model wrote, reasoning included.
    pub output_tokens: u64,
    /// The part of `output_tokens` spent on reasoning, when the backend counts
    /// it.
    pub reasoning_tokens: Option<u64>,
    /// The part of `input_tokens` read from a cache, when the backend counts
    /// it.
    pub cache_read_tokens: Option<u64>,
    /// The part of `input_tokens` written to a cache, when the backend counts
    /// it.
    pub cache_write_tokens: Option<u64>,
}

/// Typed completion failure.
///
/// Each `omnia:model` outcome maps onto the variant that names it:
///
/// - `invalid` is [`Error::InvalidRequest`];
/// - an `exhausted` completion is [`Error::BudgetExhausted`];
/// - `other` is [`Error::Backend`];
/// - every other case is the variant of its name, as `not-found` is
///   [`Error::NotFound`].
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    /// The request, or the guest's side of its session, is malformed: empty
    /// `messages`, a reserved or repeated tool name, a schema that is not
    /// JSON, or a result over the host's size cap. Retrying without changing
    /// it is pointless.
    #[error("invalid request: {0}")]
    InvalidRequest(String),
    /// Iteration, token, or time budget exhausted. When the last round ended
    /// on a rejected `check`, the detail is that correction text.
    #[error("budget exhausted: {0}")]
    BudgetExhausted(String),
    /// The deployment declares no model by the name the request opens, or the
    /// directory the request lends does not exist.
    #[error("not found: {0}")]
    NotFound(String),
    /// The provider refused the caller, or the request lends a directory the
    /// guest does not hold.
    #[error("denied: {0}")]
    Denied(String),
    /// A valid request the backend cannot serve, such as a generation control
    /// it lacks.
    #[error("unsupported: {0}")]
    Unsupported(String),
    /// The provider did not answer, or turned the call away for now as a rate
    /// limit does. A later retry may succeed.
    #[error("unavailable: {0}")]
    Unavailable(String),
    /// Any other failure of the provider or its backend.
    #[error("backend failure: {0}")]
    Backend(String),
}

impl Error {
    /// The kebab-case code, stable for callers to branch on.
    #[must_use]
    pub const fn code(&self) -> &'static str {
        match self {
            Self::InvalidRequest(_) => "invalid-request",
            Self::BudgetExhausted(_) => "budget-exhausted",
            Self::NotFound(_) => "not-found",
            Self::Denied(_) => "denied",
            Self::Unsupported(_) => "unsupported",
            Self::Unavailable(_) => "unavailable",
            Self::Backend(_) => "backend",
        }
    }
}

impl From<anyhow::Error> for Error {
    fn from(error: anyhow::Error) -> Self {
        error.downcast().unwrap_or_else(|other| Self::Backend(format!("{other:#}")))
    }
}

// The default taxonomy: the guest's own request, and findings the model could
// not resolve, are the request's, a provider that did not answer or failed is
// the gateway's, and the rest is the host's.
impl From<Error> for crate::Error {
    fn from(error: Error) -> Self {
        let code = error.code().to_owned();
        let description = error.to_string();
        match error {
            Error::InvalidRequest(_) | Error::BudgetExhausted(_) => {
                Self::BadRequest { code, description }
            }
            Error::NotFound(_) | Error::Denied(_) | Error::Unsupported(_) => {
                Self::ServerError { code, description }
            }
            Error::Unavailable(_) | Error::Backend(_) => Self::BadGateway { code, description },
        }
    }
}

/// One tool invocation the model asked the guest to run, delivered to the
/// [`Request::complete_with`] handler.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ToolCall {
    /// Correlation id the session answers by; the handler never needs it.
    pub id: String,
    /// The declared function-tool name the model called, or the reserved
    /// `check` when the request asked for one.
    pub name: String,
    /// JSON arguments object for the tool, per its declared parameters
    /// schema; the candidate text for `check`.
    pub arguments: String,
}

impl ToolCall {
    /// Deserialize the call's `arguments` into `T`.
    ///
    /// # Errors
    ///
    /// Returns model-visible failure text when the arguments do not match `T`.
    pub fn arguments<T: DeserializeOwned>(&self) -> Result<T, String> {
        serde_json::from_str(&self.arguments).map_err(|error| format!("invalid arguments: {error}"))
    }
}

impl From<Message> for types::Message {
    fn from(message: Message) -> Self {
        Self {
            role: message.role.into(),
            content: message.content,
        }
    }
}

impl From<Role> for types::Role {
    fn from(role: Role) -> Self {
        match role {
            Role::System => Self::System,
            Role::User => Self::User,
            Role::Assistant => Self::Assistant,
        }
    }
}

impl From<Generation> for types::Generation {
    fn from(generation: Generation) -> Self {
        Self {
            temperature: generation.temperature,
            top_p: generation.top_p,
            max_tokens: generation.max_tokens,
            stop_sequences: generation.stop,
            seed: generation.seed,
            effort: generation.effort.map(Into::into),
        }
    }
}

impl From<Effort> for types::Effort {
    fn from(effort: Effort) -> Self {
        match effort {
            Effort::Low => Self::Low,
            Effort::Medium => Self::Medium,
            Effort::High => Self::High,
            Effort::Other(level) => Self::Other(level),
        }
    }
}

impl From<Format> for types::Format {
    fn from(format: Format) -> Self {
        match format {
            Format::Text => Self::Text,
            Format::Json => Self::Json,
            Format::Schema(s) => Self::Schema(types::Schema {
                name: s.name,
                schema: s.schema,
            }),
        }
    }
}

impl From<Function> for types::Tool {
    fn from(function: Function) -> Self {
        Self {
            name: function.name,
            description: Some(function.description).filter(|description| !description.is_empty()),
            input_schema: function.parameters,
        }
    }
}

impl From<WorkerGrant> for types::WorkerGrant {
    fn from(grant: WorkerGrant) -> Self {
        Self {
            egress: grant.egress,
            read: grant.read,
            env: grant.env,
        }
    }
}

impl From<types::Reply> for Reply {
    fn from(reply: types::Reply) -> Self {
        Self {
            answer: reply.answer,
            usage: reply.usage.map(Into::into),
        }
    }
}

impl From<types::Usage> for Usage {
    fn from(usage: types::Usage) -> Self {
        Self {
            input_tokens: usage.input_tokens,
            output_tokens: usage.output_tokens,
            reasoning_tokens: usage.reasoning_tokens,
            cache_read_tokens: usage.cache_read_tokens,
            cache_write_tokens: usage.cache_write_tokens,
        }
    }
}

impl From<types::Error> for Error {
    fn from(error: types::Error) -> Self {
        match error {
            types::Error::Invalid(detail) => Self::InvalidRequest(detail),
            types::Error::NotFound(detail) => Self::NotFound(detail),
            types::Error::Denied(detail) => Self::Denied(detail),
            types::Error::Unsupported(detail) => Self::Unsupported(detail),
            types::Error::Unavailable(detail) => Self::Unavailable(detail),
            types::Error::Other(detail) => Self::Backend(detail),
        }
    }
}
