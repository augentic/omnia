//! Host-only types used by backends.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::host::generated::omnia::model::completion::{Effort, Mcp, Request, Role, Tool};

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

// The single-text-prompt form, for backends that steer output shape through
// prose (see `Format::instruction`): system channel first, then each message
// (non-user turns marked with their role), then the format instruction.
impl fmt::Display for Request {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut sep = if let Some(system) = &self.system {
            f.write_str(system)?;
            "\n\n"
        } else {
            ""
        };
        for message in &self.messages {
            match message.role {
                Role::User => write!(f, "{sep}{}", message.content)?,
                Role::System | Role::Assistant => {
                    write!(f, "{sep}[{}]\n{}", message.role, message.content)?;
                }
            }
            sep = "\n\n";
        }
        write!(f, "{sep}{}", self.format.instruction())
    }
}

impl Request {
    /// The request's MCP server grants, each carrying its own endpoint URL.
    #[must_use]
    pub fn mcp_servers(&self) -> Vec<&Mcp> {
        self.tools
            .iter()
            .filter_map(|tool| match tool {
                Tool::Mcp(grant) => Some(grant),
                Tool::Function(_) => None,
            })
            .collect()
    }
}

/// A backend's result: the parsed answer value, optional usage, and transcript.
///
/// Host-only — the guest sees a `reply` whose `answer` is the validated string
/// the `create` binding derives from `value`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Answer {
    /// The parsed JSON answer the backend produced.
    pub value: serde_json::Value,
    /// Token accounting the backend reported, surfaced to the guest as `reply.usage`.
    pub usage: Option<Usage>,
    /// Optional tool-call transcript the backend captured.
    pub transcript: Option<Transcript>,
}

/// Token accounting for one completion. Mirrors the WIT `usage` record; the
/// serde derive lets backends record it alongside the transcript.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Usage {
    /// Prompt tokens consumed.
    pub input_tokens: u32,
    /// Completion tokens produced.
    pub output_tokens: u32,
    /// Reasoning tokens, for models that bill them separately.
    pub reasoning_tokens: Option<u32>,
}

/// A reference an adapter asked the model to resolve (`ToolHost::resolve`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Reference {
    /// The opaque reference body the adapter's `references` shelf interprets.
    pub name: String,
}

/// One bounded directory entry returned by `ToolHost::list`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DirEntry {
    /// Entry name (never an OS path).
    pub name: String,
    /// Whether the entry is a directory.
    pub is_directory: bool,
}

/// The outcome of a `verify` profile run.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct VerifyReport {
    /// Whether the check passed.
    pub ok: bool,
    /// Human-readable detail.
    pub detail: String,
}

/// One recorded tool interaction within a completion's transcript.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolTurn {
    /// The tool the model called.
    pub tool: String,
    /// The arguments the model supplied.
    pub args: serde_json::Value,
    /// The result the host returned.
    pub result: serde_json::Value,
}

/// The tool-call transcript a backend may capture for diagnostics or future
/// replay. Host-only; it never crosses the WIT boundary. Empty for backends
/// with no tool loop (cursor).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Transcript {
    /// Ordered tool turns the backend drove to reach the answer.
    pub turns: Vec<ToolTurn>,
}

#[cfg(test)]
mod tests {
    use crate::host::generated::omnia::model::completion::{
        Format, Function, Grants, Mcp, Message, Request, Role, Tool,
    };

    fn request(system: Option<&str>, messages: Vec<Message>) -> Request {
        Request {
            model: None,
            system: system.map(str::to_owned),
            messages,
            generation: None,
            format: Format::Text,
            tools: vec![],
            grants: Grants {
                references: None,
                workspace: None,
                verify: vec![],
            },
        }
    }

    fn message(role: Role, content: &str) -> Message {
        Message {
            role,
            content: content.to_owned(),
        }
    }

    #[test]
    fn request_display_prompt() {
        let request = request(
            Some("sys"),
            vec![
                message(Role::User, "hi"),
                message(Role::Assistant, "ack"),
                message(Role::System, "note"),
            ],
        );
        let expected = format!(
            "sys\n\nhi\n\n[assistant]\nack\n\n[system]\nnote\n\n{}",
            Format::Text.instruction()
        );
        assert_eq!(request.to_string(), expected);
    }

    #[test]
    fn request_display_instruction_only() {
        assert_eq!(request(None, vec![]).to_string(), Format::Text.instruction());
    }

    #[test]
    fn mcp_servers_skips_function_tools() {
        let mut request = request(None, vec![]);
        request.tools = vec![
            Tool::Function(Function {
                name: "lookup".to_owned(),
                description: "look things up".to_owned(),
                parameters: "{}".to_owned(),
            }),
            Tool::Mcp(Mcp {
                name: "docs".to_owned(),
                tools: vec![],
                url: "http://127.0.0.1:7737/mcp/docs".to_owned(),
            }),
        ];
        let servers = request.mcp_servers();
        assert_eq!(servers.len(), 1);
        assert_eq!(servers[0].name, "docs");
    }
}
