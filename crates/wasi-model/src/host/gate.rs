//! Host-side request validation for the `create` binding.

use crate::host::Error;
use crate::host::generated::omnia::model::completion::{Format, Request, Tool};

const TOOL_NAMES: &[&str] = &["read", "list", "write"];

/// Validate a guest request before it reaches a backend.
///
/// # Errors
///
/// Returns [`Error::InvalidRequest`] when `messages` is empty, a guest tool
/// shadows a reserved host-injected tool name, a function tool's parameters
/// document is not JSON, or a `format::schema` document does not parse or
/// compile.
pub fn validate(request: &Request) -> Result<(), Error> {
    // Only guest-declared functions carry a name that could shadow a
    // host-injected tool; MCP grants name a server, not a tool.
    for tool in &request.tools {
        let Tool::Function(function) = tool else {
            continue;
        };
        if TOOL_NAMES.contains(&function.name.as_str()) {
            return Err(Error::InvalidRequest(format!("reserved tool name: {}", function.name)));
        }
        if serde_json::from_str::<serde_json::Value>(&function.parameters).is_err() {
            return Err(Error::InvalidRequest(format!(
                "function tool `{}` parameters is not valid JSON",
                function.name
            )));
        }
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

// Unit tests by design: request gating is pure validation of the request
// document, ahead of any backend.
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

    // `resolve` is no longer host-injected: a guest may declare it as an
    // ordinary function tool.
    #[test]
    fn resolve_is_an_ordinary_tool_name() {
        let mut request = request_from(vec![message(Role::User, "hi")]);
        request.tools.push(Tool::Function(Function {
            name: "resolve".to_owned(),
            description: "look up a reference".to_owned(),
            parameters: "{\"type\":\"object\"}".to_owned(),
        }));
        validate(&request).unwrap();
    }

    #[test]
    fn invalid_function_parameters() {
        let mut request = request_from(vec![message(Role::User, "hi")]);
        request.tools.push(Tool::Function(Function {
            name: "lookup".to_owned(),
            description: "look something up".to_owned(),
            parameters: "not json".to_owned(),
        }));
        let err = validate(&request).unwrap_err();
        assert!(matches!(err, Error::InvalidRequest(m) if m.contains("`lookup`")));
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
