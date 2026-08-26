//! Request validation and prompt shaping.

use std::fmt;

use crate::host::Error;
use crate::host::generated::omnia::model::completion::{Mcp, Request, Role, Tool};

const RESERVED_TOOLS: &[&str] = &["read", "list", "write"];

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

    /// The request's tool names, each carrying its own parameters schema.
    pub fn tool_names(&self) -> Vec<String> {
        self.tools
            .iter()
            .filter_map(|tool| match tool {
                Tool::Function(function) => Some(function.name.clone()),
                Tool::Mcp(_) => None,
            })
            .collect()
    }

    /// Validate the request and its tools.
    ///
    /// # Errors
    ///
    /// Returns an `InvalidRequest` error if the request is invalid.
    pub fn validate(&self) -> Result<(), Error> {
        for tool in &self.tools {
            let Tool::Function(function) = tool else {
                continue;
            };
            if RESERVED_TOOLS.contains(&function.name.as_str()) {
                return Err(Error::InvalidRequest(format!(
                    "reserved tool name: {}",
                    function.name
                )));
            }
            if serde_json::from_str::<serde_json::Value>(&function.parameters).is_err() {
                return Err(Error::InvalidRequest(format!(
                    "function tool `{}` parameters is not valid JSON",
                    function.name
                )));
            }
        }

        if self.messages.iter().all(|message| message.content.trim().is_empty()) {
            return Err(Error::InvalidRequest("empty request".to_owned()));
        }

        self.format.validate_definition().map_err(Error::InvalidRequest)
    }
}

impl fmt::Display for Request {
    // The prompt is the request's blocks joined by blank lines: the system
    // text, each message (non-user ones under a `[role]` header), and the
    // format's final-answer instruction.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut blocks: Vec<String> = self.system.iter().cloned().collect();
        blocks.extend(self.messages.iter().map(|message| match message.role {
            Role::User => message.content.clone(),
            Role::System | Role::Assistant => {
                format!("[{}]\n{}", message.role, message.content)
            }
        }));
        blocks.push(self.format.instruction());
        f.write_str(&blocks.join("\n\n"))
    }
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

#[cfg(test)]
mod tests {
    use crate::host::Error;
    use crate::host::generated::omnia::model::completion::{
        Format, Function, Grants, Message, Request, Role, Schema, Tool,
    };

    #[test]
    fn reserved_tool_name() {
        let mut request = request(vec![message(Role::User, "hi")]);
        request.tools.push(Tool::Function(Function {
            name: "read".to_owned(),
            description: "shadow a host-injected tool".to_owned(),
            parameters: "{}".to_owned(),
        }));
        let err = request.validate().unwrap_err();
        assert!(matches!(err, Error::InvalidRequest(m) if m.contains("reserved tool name")));
    }

    #[test]
    fn empty_request() {
        let err = request(vec![]).validate().unwrap_err();
        assert!(matches!(err, Error::InvalidRequest(m) if m == "empty request"));

        let err = request(vec![message(Role::User, "   ")]).validate().unwrap_err();
        assert!(matches!(err, Error::InvalidRequest(m) if m == "empty request"));
    }

    #[test]
    fn invalid_function_parameters() {
        let mut request = request(vec![message(Role::User, "hi")]);
        request.tools.push(Tool::Function(Function {
            name: "lookup".to_owned(),
            description: "look something up".to_owned(),
            parameters: "not json".to_owned(),
        }));
        let err = request.validate().unwrap_err();
        assert!(matches!(err, Error::InvalidRequest(m) if m.contains("`lookup`")));
    }

    #[test]
    fn invalid_format_schema() {
        let mut request = request(vec![message(Role::User, "hi")]);
        request.format = Format::Schema(Schema {
            name: "verdict".to_owned(),
            schema: "not json".to_owned(),
        });
        let err = request.validate().unwrap_err();
        assert!(matches!(err, Error::InvalidRequest(_)));
    }

    fn request(messages: Vec<Message>) -> Request {
        Request {
            model: None,
            system: None,
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
}
