//! Prompt-completion (model) capability.
//!
//! Target-independent mirrors of the `omnia:model/completion` records, minus
//! `tools` and `grants`: workspace lending borrows a `wasi:filesystem`
//! descriptor resource that only exists on `wasm32`, so guests needing tools
//! or grants call the raw `omnia-wasi-model` binding directly.

use std::future::Future;

use anyhow::Result;

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
#[derive(Clone, Debug)]
pub struct Message {
    /// Turn author.
    pub role: Role,
    /// Turn body text.
    pub content: String,
}

/// JSON Schema constrained output.
#[derive(Clone, Debug)]
pub struct Schema {
    /// Schema name passed to the provider (e.g. `review_result`).
    pub name: String,
    /// JSON Schema document the answer must conform to.
    pub schema: String,
}

/// Output shape constraint for the completion.
#[derive(Clone, Debug, Default)]
pub enum Format {
    /// Answer is plain text.
    #[default]
    Text,
    /// Answer must parse as a JSON object.
    Json,
    /// Answer must validate against the given JSON Schema.
    Schema(Schema),
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
#[derive(Clone, Debug, Default)]
pub struct Generation {
    /// Sampling temperature.
    pub temperature: Option<f32>,
    /// Nucleus sampling threshold.
    pub top_p: Option<f32>,
    /// Maximum output tokens.
    pub max_tokens: Option<u32>,
    /// Sequences that halt generation.
    pub stop: Vec<String>,
    /// Seed for reproducible sampling when the provider supports it.
    pub seed: Option<u64>,
    /// Reasoning-effort hint for thinking-capable models.
    pub effort: Option<Effort>,
}

/// Complete request for one completion.
#[derive(Clone, Debug, Default)]
pub struct Request {
    /// Opaque model id hint; passed through unchanged. Backend may override.
    pub model: Option<String>,
    /// System / instructions channel.
    pub system: Option<String>,
    /// Chat turns sent to the provider. Must not be empty.
    pub messages: Vec<Message>,
    /// Sampling and length controls.
    pub generation: Option<Generation>,
    /// Required output shape and validation rules.
    pub format: Format,
}

/// Token accounting for one completion, when the backend reports it.
#[derive(Clone, Copy, Debug)]
pub struct Usage {
    /// Prompt tokens consumed.
    pub input_tokens: u32,
    /// Completion tokens produced.
    pub output_tokens: u32,
    /// Reasoning tokens, for models that bill them separately.
    pub reasoning_tokens: Option<u32>,
}

/// One validated completion result.
#[derive(Clone, Debug)]
pub struct Reply {
    /// The validated answer, per [`Request::format`](Request).
    pub answer: String,
    /// Token accounting, when the backend reports it.
    pub usage: Option<Usage>,
}

/// Prompt completion (Omnia Model).
///
/// Default WASM implementations delegate to `omnia:model/completion` via
/// `omnia-wasi-model`.
pub trait Model: Send + Sync {
    /// Single-shot completion returning one validated reply.
    #[cfg(not(target_arch = "wasm32"))]
    fn complete(&self, request: Request) -> impl Future<Output = Result<Reply>> + Send;

    /// Single-shot completion returning one validated reply.
    #[cfg(target_arch = "wasm32")]
    fn complete(&self, request: Request) -> impl Future<Output = Result<Reply>> + Send {
        use anyhow::anyhow;
        use omnia_wasi_model::completion;

        async move {
            let request = completion::Request {
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
                tools: vec![],
                grants: completion::Grants {
                    references: None,
                    workspace: None,
                    verify: vec![],
                },
            };
            let reply = completion::create(request)
                .await
                .map_err(|e| anyhow!("creating completion: {e:?}"))?;
            Ok(Reply {
                answer: reply.answer,
                usage: reply.usage.map(|u| Usage {
                    input_tokens: u.input_tokens,
                    output_tokens: u.output_tokens,
                    reasoning_tokens: u.reasoning_tokens,
                }),
            })
        }
    }
}
