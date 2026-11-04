//! # WASI Model Guest

// Bindings for the `omnia:model` world.
mod generated {
    #![allow(missing_docs, clippy::same_length_and_capacity, reason = "wit-bindgen output")]
    wit_bindgen::generate!({
        world: "model",
        path: "wit",
        with: {
            "wasi:filesystem/types@0.3.2": wasip3::filesystem::types,
            "wasi:clocks/system-clock@0.3.2": wasip3::clocks::system_clock,
            "wasi:clocks/types@0.3.2": wasip3::clocks::types,
        },
    });
}

pub use self::generated::omnia::model::*;
pub use self::generated::{wit_future, wit_stream};

impl From<omnia_backend::model::Message> for completion::Message {
    fn from(message: omnia_backend::model::Message) -> Self {
        Self {
            role: message.role.into(),
            content: message.content,
        }
    }
}

impl From<omnia_backend::model::Role> for completion::Role {
    fn from(role: omnia_backend::model::Role) -> Self {
        match role {
            omnia_backend::model::Role::System => Self::System,
            omnia_backend::model::Role::User => Self::User,
            omnia_backend::model::Role::Assistant => Self::Assistant,
        }
    }
}

impl From<omnia_backend::model::Generation> for completion::Generation {
    fn from(generation: omnia_backend::model::Generation) -> Self {
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

impl From<omnia_backend::model::Effort> for completion::Effort {
    fn from(effort: omnia_backend::model::Effort) -> Self {
        match effort {
            omnia_backend::model::Effort::Minimal => Self::Minimal,
            omnia_backend::model::Effort::Low => Self::Low,
            omnia_backend::model::Effort::Medium => Self::Medium,
            omnia_backend::model::Effort::High => Self::High,
        }
    }
}

impl From<omnia_backend::model::Format> for completion::Format {
    fn from(format: omnia_backend::model::Format) -> Self {
        match format {
            omnia_backend::model::Format::Text => Self::Text,
            omnia_backend::model::Format::Json => Self::Json,
            omnia_backend::model::Format::Schema(s) => Self::Schema(completion::Schema {
                name: s.name,
                schema: s.schema,
            }),
        }
    }
}

impl From<omnia_backend::model::Tool> for completion::Tool {
    fn from(tool: omnia_backend::model::Tool) -> Self {
        match tool {
            omnia_backend::model::Tool::Function(omnia_backend::model::Function {
                name,
                description,
                parameters,
            }) => Self::Function(completion::Function {
                name,
                description,
                parameters,
            }),
            omnia_backend::model::Tool::Mcp(omnia_backend::model::McpGrant {
                name,
                tools,
                url,
            }) => Self::Mcp(completion::Mcp { name, tools, url }),
        }
    }
}

impl From<omnia_backend::model::WorkerGrant> for completion::WorkerGrant {
    fn from(grant: omnia_backend::model::WorkerGrant) -> Self {
        Self {
            egress: grant.egress,
            read: grant.read,
            env: grant.env,
        }
    }
}

impl From<completion::Reply> for omnia_backend::model::Reply {
    fn from(reply: completion::Reply) -> Self {
        Self {
            answer: reply.answer,
            usage: reply.usage.map(Into::into),
        }
    }
}

impl From<completion::Usage> for omnia_backend::model::Usage {
    fn from(usage: completion::Usage) -> Self {
        Self {
            input_tokens: usage.input_tokens,
            output_tokens: usage.output_tokens,
            reasoning_tokens: usage.reasoning_tokens,
        }
    }
}

impl From<completion::Error> for omnia_backend::model::Error {
    fn from(error: completion::Error) -> Self {
        match error {
            completion::Error::InvalidRequest(detail) => Self::InvalidRequest(detail),
            completion::Error::BudgetExhausted(detail) => Self::BudgetExhausted(detail),
            completion::Error::ToolFailed(detail) => Self::ToolFailed(detail),
            completion::Error::Backend(detail) => Self::Backend(detail),
        }
    }
}
