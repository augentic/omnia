//! Canonical JSON for replay fixture keys (§5.4).

use std::fmt;

use serde_json::{Value, json};

use crate::host::generated::omnia::model::completion::{Effort, Format, Role, Tool, ToolChoice};

impl fmt::Display for Role {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::System => "system",
            Self::User => "user",
            Self::Assistant => "assistant",
        })
    }
}

impl fmt::Display for Effort {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Minimal => "minimal",
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
        })
    }
}

impl Format {
    pub(crate) fn replay_value(&self) -> Value {
        match self {
            Self::Text => json!({ "kind": "text" }),
            Self::Json => json!({ "kind": "json" }),
            Self::Schema(spec) => json!({
                "kind": "schema",
                "schema": {
                    "name": spec.name,
                    "schema": spec.schema,
                    "strict": spec.strict,
                },
            }),
        }
    }
}

impl Tool {
    pub(crate) fn replay_value(&self) -> Value {
        match self {
            Self::Function(function) => json!({
                "function": {
                    "name": function.name,
                    "description": function.description,
                    "parameters": function.parameters,
                },
            }),
            Self::Mcp(mcp) => json!({
                "mcp": {
                    "name": mcp.name,
                    "tools": mcp.tools,
                },
            }),
        }
    }
}

impl ToolChoice {
    pub(crate) fn replay_value(&self) -> Value {
        match self {
            Self::Auto => json!("auto"),
            Self::None => json!("none"),
            Self::Required => json!("required"),
            Self::Named(name) => json!({ "named": name }),
        }
    }
}
