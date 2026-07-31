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
    /// Iteration, token, or time budget exhausted.
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
                messages: request.messages.into_iter().map(Into::into).collect(),
                generation: request.generation.map(Into::into),
                format: request.format.into(),
                tools: request.tools.into_iter().map(Into::into).collect(),
                grants: completion::Grants {
                    references: request.references,
                    workspace,
                },
            };

            completion::create(wire).await.map(Into::into).map_err(Into::into)
        }
    }
}

/// Mirror-to-wire conversions between the target-independent records above
/// and the `omnia:model/completion` bindings.
#[cfg(target_arch = "wasm32")]
mod wire {
    use omnia_wasi_model::completion;

    use super::{
        Effort, Error, Format, Function, Generation, McpGrant, Message, Reply, Role, Tool, Usage,
    };

    impl From<Role> for completion::Role {
        fn from(role: Role) -> Self {
            match role {
                Role::System => Self::System,
                Role::User => Self::User,
                Role::Assistant => Self::Assistant,
            }
        }
    }

    impl From<Message> for completion::Message {
        fn from(message: Message) -> Self {
            Self {
                role: message.role.into(),
                content: message.content,
            }
        }
    }

    impl From<Effort> for completion::Effort {
        fn from(effort: Effort) -> Self {
            match effort {
                Effort::Minimal => Self::Minimal,
                Effort::Low => Self::Low,
                Effort::Medium => Self::Medium,
                Effort::High => Self::High,
            }
        }
    }

    impl From<Generation> for completion::Generation {
        fn from(generation: Generation) -> Self {
            Self {
                temperature: generation.temperature,
                top_p: generation.top_p,
                max_tokens: generation.max_tokens,
                stop: generation.stop,
                seed: generation.seed,
                effort: generation.effort.map(Into::into),
            }
        }
    }

    impl From<Format> for completion::Format {
        fn from(format: Format) -> Self {
            match format {
                Format::Text => Self::Text,
                Format::Json => Self::Json,
                Format::Schema(s) => Self::Schema(completion::Schema {
                    name: s.name,
                    schema: s.schema,
                }),
            }
        }
    }

    impl From<Tool> for completion::Tool {
        fn from(tool: Tool) -> Self {
            match tool {
                Tool::Function(Function {
                    name,
                    description,
                    parameters,
                }) => Self::Function(completion::Function {
                    name,
                    description,
                    parameters,
                }),
                Tool::Mcp(McpGrant { name, tools, url }) => {
                    Self::Mcp(completion::Mcp { name, tools, url })
                }
            }
        }
    }

    impl From<completion::Usage> for Usage {
        fn from(usage: completion::Usage) -> Self {
            Self {
                input_tokens: usage.input_tokens,
                output_tokens: usage.output_tokens,
                reasoning_tokens: usage.reasoning_tokens,
            }
        }
    }

    impl From<completion::Reply> for Reply {
        fn from(reply: completion::Reply) -> Self {
            Self {
                answer: reply.answer,
                usage: reply.usage.map(Into::into),
            }
        }
    }

    impl From<completion::Error> for Error {
        fn from(error: completion::Error) -> Self {
            match error {
                completion::Error::InvalidRequest(detail) => Self::InvalidRequest(detail),
                completion::Error::InvalidAnswer(detail) => Self::InvalidAnswer(detail),
                completion::Error::BudgetExhausted(detail) => Self::BudgetExhausted(detail),
                completion::Error::ToolFailed(detail) => Self::ToolFailed(detail),
                completion::Error::Backend(detail) => Self::Backend(detail),
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
