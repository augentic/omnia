//! Request validation and prompt shaping.

use std::fmt;

use crate::host::Error;
use crate::host::generated::omnia::model::completion::{Mcp, Request, Role, Tool};

const RESERVED_TOOLS: &[&str] = &["read", "list", "write"];

impl fmt::Display for Role {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::System => "system",
            Self::User => "user",
            Self::Assistant => "assistant",
        })
    }
}

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

    pub(super) fn function_names(&self) -> Vec<String> {
        self.tools
            .iter()
            .filter_map(|tool| match tool {
                Tool::Function(function) => Some(function.name.clone()),
                Tool::Mcp(_) => None,
            })
            .collect()
    }
}

pub(super) fn validate(request: &Request) -> Result<(), Error> {
    for tool in &request.tools {
        let Tool::Function(function) = tool else {
            continue;
        };
        if RESERVED_TOOLS.contains(&function.name.as_str()) {
            return Err(Error::InvalidRequest(format!("reserved tool name: {}", function.name)));
        }
        if serde_json::from_str::<serde_json::Value>(&function.parameters).is_err() {
            return Err(Error::InvalidRequest(format!(
                "function tool `{}` parameters is not valid JSON",
                function.name
            )));
        }
    }

    if request.messages.iter().all(|message| message.content.trim().is_empty()) {
        return Err(Error::InvalidRequest("empty request".to_owned()));
    }

    request.format.validate_definition().map_err(Error::InvalidRequest)
}

#[cfg(test)]
mod tests {
    use super::validate;
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
        let err = validate(&request).unwrap_err();
        assert!(matches!(err, Error::InvalidRequest(m) if m.contains("reserved tool name")));
    }

    #[test]
    fn empty_request() {
        let err = validate(&request(vec![])).unwrap_err();
        assert!(matches!(err, Error::InvalidRequest(m) if m == "empty request"));

        let err = validate(&request(vec![message(Role::User, "   ")])).unwrap_err();
        assert!(matches!(err, Error::InvalidRequest(m) if m == "empty request"));
    }

    #[test]
    fn ordinary_function_name() {
        let mut request = request(vec![message(Role::User, "hi")]);
        request.tools.push(Tool::Function(Function {
            name: "resolve".to_owned(),
            description: "look up a reference".to_owned(),
            parameters: "{\"type\":\"object\"}".to_owned(),
        }));
        validate(&request).unwrap();
    }

    #[test]
    fn invalid_function_parameters() {
        let mut request = request(vec![message(Role::User, "hi")]);
        request.tools.push(Tool::Function(Function {
            name: "lookup".to_owned(),
            description: "look something up".to_owned(),
            parameters: "not json".to_owned(),
        }));
        let err = validate(&request).unwrap_err();
        assert!(matches!(err, Error::InvalidRequest(m) if m.contains("`lookup`")));
    }

    #[test]
    fn invalid_format_schema() {
        let mut request = request(vec![message(Role::User, "hi")]);
        request.format = Format::Schema(Schema {
            name: "verdict".to_owned(),
            schema: "not json".to_owned(),
        });
        let err = validate(&request).unwrap_err();
        assert!(matches!(err, Error::InvalidRequest(m) if m.contains("not valid JSON")));

        request.format = Format::Schema(Schema {
            name: "verdict".to_owned(),
            schema: "{\"type\":\"nonsense\"}".to_owned(),
        });
        let err = validate(&request).unwrap_err();
        assert!(matches!(err, Error::InvalidRequest(m) if m.contains("valid JSON Schema")));
    }

    #[test]
    fn chat_prompt() {
        let request = Request {
            system: Some("sys".to_owned()),
            messages: vec![
                message(Role::User, "hi"),
                message(Role::Assistant, "ack"),
                message(Role::System, "note"),
            ],
            ..request(vec![])
        };
        let expected = format!(
            "sys\n\nhi\n\n[assistant]\nack\n\n[system]\nnote\n\n{}",
            Format::Text.instruction()
        );
        assert_eq!(request.to_string(), expected);
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
