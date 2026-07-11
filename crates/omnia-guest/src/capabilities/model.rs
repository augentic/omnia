//! Prompt-completion (model) capability.
//!
//! Target-independent mirrors of the `omnia:model/completion` records. The
//! one record that cannot cross off `wasm32` is the `grants.workspace`
//! descriptor lend — a `wasi:filesystem` resource that only exists on
//! `wasm32` — so a guest asks for it with the plain
//! [`Request::lend_workspace`] flag and the `wasm32` default body resolves
//! it against the guest's own `"."` preopen at the call site.

use std::future::Future;

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

/// One chat turn passed to the provider API.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Message {
    /// Turn author.
    pub role: Role,
    /// Turn body text.
    pub content: String,
}

/// JSON Schema constrained output.
#[derive(Clone, Debug, PartialEq, Eq, bon::Builder)]
pub struct SchemaFormat {
    /// Schema name passed to the provider (e.g. `review_result`).
    #[builder(into)]
    pub name: String,
    /// JSON Schema document the answer must conform to.
    #[builder(into)]
    pub schema: String,
}

/// Output shape constraint for the completion.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Format {
    /// Answer is plain text.
    #[default]
    Text,
    /// Answer must parse as a JSON object.
    Json,
    /// Answer must validate against the given JSON Schema; the host enforces
    /// this at the `create` gate.
    Schema(SchemaFormat),
}

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

/// Guest-declared function tool advertised to the model.
#[derive(Clone, Debug, PartialEq, Eq, bon::Builder)]
pub struct Function {
    /// Tool name. Must not collide with reserved host-injected tool names
    /// (`resolve`, `read`, …).
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

/// A tool offered to the model: a guest-declared function or an MCP server
/// grant carrying its own endpoint URL.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Tool {
    /// Guest-declared function tool.
    Function(Function),
    /// MCP server grant.
    Mcp(McpGrant),
}

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
    /// Required output shape and validation rules.
    #[builder(default)]
    pub format: Format,
    /// Guest-declared functions and MCP grants merged with host-injected
    /// tools at the backend.
    #[builder(default)]
    pub tools: Vec<Tool>,
    /// Guest id whose `references` export the host-injected `resolve` tool
    /// targets (`grants.references`).
    #[builder(into)]
    pub references: Option<String>,
    /// Allowed closed verification profile names (`grants.verify`).
    #[builder(default)]
    pub verify: Vec<String>,
    /// Lend the guest's `"."` preopen through `grants.workspace`, giving the
    /// backend (and any spawned agent) the shared project mount.
    #[builder(default)]
    pub lend_workspace: bool,
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

/// One validated completion result.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Reply {
    /// The validated answer, per [`Request::format`](Request).
    pub answer: String,
    /// Token accounting, when the backend reports it.
    pub usage: Option<Usage>,
}

/// Typed completion failure, mirroring the `omnia:model` error variant.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    /// The request itself is malformed (empty `messages`, reserved tool
    /// name, invalid schema document); retrying without changing it is
    /// pointless.
    #[error("invalid request: {0}")]
    InvalidRequest(String),
    /// Backend produced output that never passed validation.
    #[error("invalid answer: {0}")]
    InvalidAnswer(String),
    /// Iteration, token, time, or verify budget exhausted.
    #[error("budget exhausted: {0}")]
    BudgetExhausted(String),
    /// Non-repairable tool error.
    #[error("tool failed: {0}")]
    ToolFailed(String),
    /// Transport, process, or provider failure.
    #[error("backend failure: {0}")]
    Backend(String),
}

/// Prompt completion (Omnia Model).
///
/// Default WASM implementations delegate to `omnia:model/completion` via
/// `omnia-wasi-model`; off `wasm32` the signature is bare so hosts and tests
/// supply their own provider.
pub trait Model: Send + Sync {
    /// Single-shot completion returning one validated reply.
    #[cfg(not(target_arch = "wasm32"))]
    fn create(&self, request: Request) -> impl Future<Output = Result<Reply, Error>> + Send;

    /// Single-shot completion returning one validated reply.
    #[cfg(target_arch = "wasm32")]
    fn create(&self, request: Request) -> impl Future<Output = Result<Reply, Error>> + Send {
        use omnia_wasi_model::completion;
        use wasip3::filesystem::preopens;

        async move {
            // The lent workspace borrows one of these descriptors, so the
            // table must outlive the `create` call below.
            let directories =
                if request.lend_workspace { preopens::get_directories() } else { vec![] };
            let workspace = directories.iter().find_map(|(dir, name)| (name == ".").then_some(dir));
            if request.lend_workspace && workspace.is_none() {
                return Err(Error::InvalidRequest(
                    "workspace lend requested but the `.` preopen is absent".to_string(),
                ));
            }

            let wire = completion::Request {
                model: request.model,
                system: request.system,
                messages: request
                    .messages
                    .into_iter()
                    .map(|m| completion::Message {
                        role: match m.role {
                            Role::System => completion::Role::System,
                            Role::User => completion::Role::User,
                            Role::Assistant => completion::Role::Assistant,
                        },
                        content: m.content,
                    })
                    .collect(),
                generation: request.generation.map(|g| completion::Generation {
                    temperature: g.temperature,
                    top_p: g.top_p,
                    max_tokens: g.max_tokens,
                    stop: g.stop,
                    seed: g.seed,
                    effort: g.effort.map(|e| match e {
                        Effort::Minimal => completion::Effort::Minimal,
                        Effort::Low => completion::Effort::Low,
                        Effort::Medium => completion::Effort::Medium,
                        Effort::High => completion::Effort::High,
                    }),
                }),
                format: match request.format {
                    Format::Text => completion::Format::Text,
                    Format::Json => completion::Format::Json,
                    Format::Schema(s) => completion::Format::Schema(completion::Schema {
                        name: s.name,
                        schema: s.schema,
                    }),
                },
                tools: request
                    .tools
                    .into_iter()
                    .map(|tool| match tool {
                        Tool::Function(f) => completion::Tool::Function(completion::Function {
                            name: f.name,
                            description: f.description,
                            parameters: f.parameters,
                        }),
                        Tool::Mcp(m) => completion::Tool::Mcp(completion::Mcp {
                            name: m.name,
                            tools: m.tools,
                            url: m.url,
                        }),
                    })
                    .collect(),
                grants: completion::Grants {
                    references: request.references,
                    workspace,
                    verify: request.verify,
                },
            };

            match completion::create(wire).await {
                Ok(reply) => Ok(Reply {
                    answer: reply.answer,
                    usage: reply.usage.map(|u| Usage {
                        input_tokens: u.input_tokens,
                        output_tokens: u.output_tokens,
                        reasoning_tokens: u.reasoning_tokens,
                    }),
                }),
                Err(completion::Error::InvalidRequest(detail)) => {
                    Err(Error::InvalidRequest(detail))
                }
                Err(completion::Error::InvalidAnswer(detail)) => Err(Error::InvalidAnswer(detail)),
                Err(completion::Error::BudgetExhausted(detail)) => {
                    Err(Error::BudgetExhausted(detail))
                }
                Err(completion::Error::ToolFailed(detail)) => Err(Error::ToolFailed(detail)),
                Err(completion::Error::Backend(detail)) => Err(Error::Backend(detail)),
            }
        }
    }
}

/// The WASI-backed provider a `wasm32` guest hands its wasm-free core; the
/// default method body carries the whole delegation.
#[cfg(target_arch = "wasm32")]
#[derive(Clone, Copy, Debug)]
pub struct WasiModel;

#[cfg(target_arch = "wasm32")]
impl Model for WasiModel {}
