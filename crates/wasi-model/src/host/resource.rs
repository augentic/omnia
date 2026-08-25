use std::fmt;
use std::time::Duration;

pub use omnia::FutureResult;
use serde::{Deserialize, Serialize};

use crate::host::generated::omnia::model::completion::{Mcp, Request, Role, Tool};

/// Host-side capabilities for one completion, lent to backends that need them.
pub trait ToolHost: Send + Sync {
    /// Run one declared function tool through the completion's session: the
    /// guest's tool closure answers. The outer error is a hard host failure
    /// (undeclared tool, exhausted budget, closed session, oversize result,
    /// timeout); the inner `Err` is the tool's own model-visible failure
    /// text, fed back to the model as repairable content.
    fn call_tool(&self, name: String, arguments: String) -> FutureResult<Result<String, String>>;

    /// Bounded workspace read via the lent `wasi:filesystem` capability.
    fn read(&self, path: String) -> FutureResult<Vec<u8>>;

    /// Bounded workspace listing via the lent `wasi:filesystem` capability.
    fn list(&self, path: String) -> FutureResult<Vec<DirEntry>>;

    /// Accumulate an edit against the session's base tree.
    fn write(&self, path: String, bytes: Vec<u8>) -> FutureResult<()>;

    /// The absolute host path of the lent workspace, when one was lent for
    /// this completion and resolved to an authorized mount.
    fn local_path(&self) -> Option<&std::path::Path> {
        None
    }
}

/// Session bounds the host enforces per completion, in `wasi:model`,
/// regardless of backend.
#[derive(Clone, Copy, Debug)]
pub struct SessionLimits {
    /// Tool calls one completion may issue before `budget-exhausted`.
    pub max_tool_calls: u32,
    /// Byte cap on a single tool result's output.
    pub max_result_bytes: usize,
    /// How long the host waits for the guest to answer one tool call.
    pub tool_timeout: Duration,
}

impl Default for SessionLimits {
    fn default() -> Self {
        Self {
            max_tool_calls: 32,
            max_result_bytes: 1 << 20,
            tool_timeout: Duration::from_secs(60),
        }
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

/// One bounded directory entry returned by `ToolHost::list`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DirEntry {
    /// Entry name (never an OS path).
    pub name: String,
    /// Whether the entry is a directory.
    pub is_directory: bool,
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

impl fmt::Display for Role {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::System => "system",
            Self::User => "user",
            Self::Assistant => "assistant",
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

#[cfg(test)]
mod tests {
    use crate::host::generated::omnia::model::completion::{
        Format, Grants, Message, Request, Role,
    };

    fn request(system: Option<&str>, messages: Vec<Message>) -> Request {
        Request {
            model: None,
            system: system.map(str::to_owned),
            messages,
            generation: None,
            format: Format::Text,
            tools: vec![],
            grants: Grants { workspace: None },
        }
    }

    fn message(role: Role, content: &str) -> Message {
        Message {
            role,
            content: content.to_owned(),
        }
    }

    #[test]
    fn display_prompt() {
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
}
