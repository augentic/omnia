//! Prompt-completion (model) service contract and value types.

mod answer;
#[cfg(not(target_arch = "wasm32"))]
mod client;
#[cfg(all(not(target_arch = "wasm32"), feature = "model-default"))]
mod default;
mod request;
#[cfg(not(target_arch = "wasm32"))]
mod session;
#[cfg(not(target_arch = "wasm32"))]
mod workspace;

use std::time::Duration;

use schemars::JsonSchema;
use serde::Serialize;
use serde::de::DeserializeOwned;

pub use self::answer::{Answer, ToolTurn, Transcript};
#[cfg(not(target_arch = "wasm32"))]
pub use self::client::ModelClient;
#[cfg(all(not(target_arch = "wasm32"), feature = "model-default"))]
pub use self::default::ModelDefault;
#[cfg(not(target_arch = "wasm32"))]
pub use self::session::{ToolHost, ToolSession};
#[cfg(not(target_arch = "wasm32"))]
pub use self::workspace::Workspace;

const MAX_RESULT_BYTES: usize = 1024 * 1024;

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
    /// at the handler as a [`ToolCall`] named `check` whose `arguments` are
    /// the candidate text. `Ok` accepts it as the reply; `Err(text)` is
    /// appended to the conversation verbatim as the correction turn and the
    /// backend goes round again.
    #[builder(default)]
    pub check: bool,
    /// Deployment-local path of the directory to lend through
    /// `grants.workspace`, giving the backend (and any spawned agent) that
    /// directory. The provider resolves the path against its authorized mounts
    /// (preopens on `wasm32`, `Direct::with_mounts` natively) — `"."` lends the
    /// shared project mount, `"/mount/sub"` lends a subdirectory of `/mount` —
    /// while test doubles observe the raw string. `None` lends nothing.
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

/// Alias matching the `omnia:model/completion` WIT record name.
pub type Schema = SchemaFormat;

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

/// Alias matching the `omnia:model/completion` WIT record name.
pub type Mcp = McpGrant;

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

/// One tool invocation the model asked the guest to run.
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

fn schema_of<T: JsonSchema>() -> String {
    schemars::schema_for!(T).to_value().to_string()
}

/// Session bounds enforced per completion, regardless of backend.
#[derive(Clone, Copy, Debug)]
pub struct Limits {
    /// Tool calls one completion may issue before `budget-exhausted`.
    pub max_tool_calls: u32,
    /// Byte cap on a single tool result's output, and on a `check`'s
    /// correction text.
    pub max_result_bytes: usize,
    /// How long the host waits for the guest to answer one tool call.
    pub tool_timeout: Duration,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_tool_calls: 32,
            max_result_bytes: MAX_RESULT_BYTES,
            tool_timeout: Duration::from_secs(60),
        }
    }
}

/// A function tool's model-visible output or failure text.
pub type ToolOutcome = Result<String, String>;

/// One bounded directory entry returned by `ToolHost::list`; backends
/// serialise the listing for the model.
#[derive(Clone, Debug, Serialize)]
pub struct DirEntry {
    /// Entry name (never an OS path).
    pub name: String,
    /// Whether the entry is a directory.
    pub is_directory: bool,
}
