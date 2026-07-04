//! Host-side request validation for the `create` binding.

use crate::host::Error;
use crate::host::generated::omnia::model::completion::{Format, Request, Tool};

const TOOL_NAMES: &[&str] = &["resolve", "read", "list", "write", "verify"];

/// Validate a guest request before it reaches a backend.
///
/// # Errors
///
/// Returns [`Error::InvalidRequest`] when `messages` is empty, a guest tool
/// shadows a reserved host-injected tool name, or a `format::schema` document
/// does not parse or compile.
pub fn validate(request: &Request) -> Result<(), Error> {
    // Only guest-declared functions carry a name that could shadow a
    // host-injected tool; MCP grants name a server, not a tool.
    if let Some(name) = request.tools.iter().find_map(|t| match t {
        Tool::Function(f) if TOOL_NAMES.contains(&f.name.as_str()) => Some(f.name.as_str()),
        _ => None,
    }) {
        return Err(Error::InvalidRequest(format!("reserved tool name: {name}")));
    }

    if request.messages.iter().all(|m| m.content.trim().is_empty()) {
        return Err(Error::InvalidRequest("empty request".to_owned()));
    }

    // Reject an uncompilable schema here so backends and the answer gate can
    // assume the document is sound.
    if let Format::Schema(spec) = &request.format {
        let schema: serde_json::Value = serde_json::from_str(&spec.schema)
            .map_err(|e| Error::InvalidRequest(format!("format schema is not valid JSON: {e}")))?;
        jsonschema::validator_for(&schema).map_err(|e| {
            Error::InvalidRequest(format!("format schema is not a valid JSON Schema: {e}"))
        })?;
    }

    Ok(())
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
        let mut request = request_from(vec![message(Role::User, "hi")]);
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
        let err = validate(&request_from(vec![])).unwrap_err();
        assert!(matches!(err, Error::InvalidRequest(m) if m == "empty request"));

        // messages present but all blank is still empty.
        let err = validate(&request_from(vec![message(Role::User, "   ")])).unwrap_err();
        assert!(matches!(err, Error::InvalidRequest(m) if m == "empty request"));
    }

    #[test]
    fn non_empty() {
        validate(&request_from(vec![message(Role::User, "hi")])).unwrap();
    }

    #[test]
    fn invalid_schema_document() {
        let mut request = request_from(vec![message(Role::User, "hi")]);
        request.format = Format::Schema(Schema {
            name: "verdict".to_owned(),
            schema: "not json".to_owned(),
        });
        let err = validate(&request).unwrap_err();
        assert!(matches!(err, Error::InvalidRequest(m) if m.contains("not valid JSON")));

        let mut request = request_from(vec![message(Role::User, "hi")]);
        request.format = Format::Schema(Schema {
            name: "verdict".to_owned(),
            schema: "{\"type\":\"nonsense\"}".to_owned(),
        });
        let err = validate(&request).unwrap_err();
        assert!(matches!(err, Error::InvalidRequest(m) if m.contains("valid JSON Schema")));
    }

    fn request_from(messages: Vec<Message>) -> Request {
        Request {
            model: None,
            system: None,
            messages,
            generation: None,
            format: Format::Json,
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
}
