use std::fmt;

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
