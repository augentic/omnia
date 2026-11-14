//! Prompt-completion (model) capability.
//!
//! Target-independent mirrors of the `omnia:model/completion` records. The
//! one record that cannot cross off `wasm32` is the `grants.workspace`
//! descriptor lend — a `wasi:filesystem` resource that only exists on
//! `wasm32` — so a guest names the directory with the plain
//! [`Request::workspace`] path and `WasiProvider` resolves it against the
//! guest's preopens at the call site, by the lend rule every lending
//! capability shares: the longest preopen whose name prefixes the path becomes
//! the lent root descriptor and the remainder rides as the grant's subpath.
//!
//! Acceptance of an answer is the guest's: a request with [`Request::check`]
//! set receives each candidate as a tool call named `check` and answers
//! `Ok` to finish or `Err(correction)` to send the model round again. The
//! typed [`Question`] runs that exchange for a `JsonSchema + Deserialize`
//! answer type.

mod question;

use std::future::Future;

use schemars::JsonSchema;
use serde::de::DeserializeOwned;

pub use self::question::{Findings, Question, ToolFuture, Tools};

/// Prompt completion (Omnia Model).
///
/// `WasiProvider` delegates to `omnia:model/completion` via `omnia-wasi-model`,
/// opening the completion session and answering the model's tool calls with
/// the supplied closure.
#[omnia_guest_macros::capability]
pub trait Model: Send + Sync {
    /// Single-shot completion returning one reply. Any tool call the model
    /// issues fails back to it; declare tools (or set `check`) and answer
    /// them through [`Model::complete_with`] instead.
    fn complete(&self, request: Request) -> impl Future<Output = Result<Reply, Error>> + Send {
        self.complete_with(request, |call: ToolCall| async move {
            Err::<String, String>(format!(
                "tool `{}` has no handler: answer tool calls with complete_with",
                call.name
            ))
        })
    }

    /// Completion with a tool closure: the model's tool calls arrive as
    /// [`ToolCall`] values answered serially by `handler` (over the caller's
    /// own locals and authority), and each result feeds the same model turn.
    /// Results correlate by id, so parallel handling stays available through
    /// the raw `omnia_wasi_model::completion` session bindings.
    fn complete_with<H, F>(
        &self, request: Request, handler: H,
    ) -> impl Future<Output = Result<Reply, Error>> + Send
    where
        H: FnMut(ToolCall) -> F + Send,
        F: Future<Output = Result<String, String>> + Send;
}

/// The reserved tool name a `check` candidate arrives under.
pub const CHECK_TOOL: &str = "check";

/// Complete request for one completion.
#[derive(Clone, Debug, Default, PartialEq, Eq, bon::Builder)]
pub struct Request {
    /// Opaque model id hint; passed through unchanged. Backend may override.
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
    /// Guest-declared functions and MCP grants merged with host-injected
    /// tools at the backend. The names `read`, `list`, `write`, and `check`
    /// are reserved.
    #[builder(default)]
    pub tools: Vec<Tool>,
    /// Ask the guest to accept each candidate answer: the candidate arrives
    /// at the [`Model::complete_with`] handler as a [`ToolCall`] named
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
    /// its lend, through `grants.worker`. Requires [`Request::workspace`]:
    /// a worker reaches beyond a lend it has. `None` grants nothing beyond
    /// the lend.
    pub worker: Option<WorkerGrant>,
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
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Effort {
    /// Least reasoning; lowest latency and cost.
    Minimal,
    /// Reduced reasoning.
    Low,
    /// Balanced reasoning.
    Medium,
    /// Most reasoning; highest latency and cost.
    High,
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

/// A tool offered to the model: a guest-declared function or an MCP server
/// grant carrying its own endpoint URL.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Tool {
    /// Guest-declared function tool.
    Function(Function),
    /// MCP server grant.
    Mcp(McpGrant),
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

/// Remote MCP server offered to the model for this completion.
#[derive(Clone, Debug, PartialEq, Eq, bon::Builder)]
pub struct McpGrant {
    /// Logical server name identifying the server (e.g. in `.cursor/mcp.json`).
    #[builder(into)]
    pub name: String,
    /// Tool allowlist; empty exposes every tool the server advertises.
    #[builder(default)]
    pub tools: Vec<String>,
    /// MCP server endpoint URL.
    #[builder(into)]
    pub url: String,
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

/// Token accounting for one completion, when the backend reports it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Usage {
    /// Prompt tokens consumed.
    pub input_tokens: u32,
    /// Completion tokens produced.
    pub output_tokens: u32,
    /// Reasoning tokens, for models that bill them separately.
    pub reasoning_tokens: Option<u32>,
}

/// Typed completion failure, mirroring the `omnia:model` error variant.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    /// The request itself is malformed (empty `messages`, reserved tool
    /// name, schema that is not JSON); retrying without changing it is
    /// pointless.
    #[error("invalid request: {0}")]
    InvalidRequest(String),
    /// Iteration, token, or time budget exhausted. When the last round ended
    /// on a rejected `check`, the detail is that correction text.
    #[error("budget exhausted: {0}")]
    BudgetExhausted(String),
    /// Non-repairable tool error.
    #[error("tool failed: {0}")]
    ToolFailed(String),
    /// Transport, process, or provider failure.
    #[error("backend failure: {0}")]
    Backend(String),
}

impl From<anyhow::Error> for Error {
    fn from(error: anyhow::Error) -> Self {
        error.downcast().unwrap_or_else(|other| Self::Backend(format!("{other:#}")))
    }
}

/// One tool invocation the model asked the guest to run, delivered to the
/// [`Model::complete_with`] handler.
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
