//! Stateless MCP JSON-RPC dispatch, independent of any transport.

use serde_json::{Value, json};

use super::McpServer;
use super::types::{INVALID_REQUEST, McpError, PARSE_ERROR};

/// The MCP protocol revision advertised when a client requests none.
pub const PROTOCOL_VERSION: &str = "2025-06-18";

/// Handle one JSON-RPC message, returning the serialized response — or `None`
/// for a notification (a message with no `id`), which never gets a reply.
///
/// The dispatch holds no state between calls, so a caller may serve each
/// message on a freshly instantiated server.
#[must_use]
pub fn handle_message(server: &dyn McpServer, body: &str) -> Option<String> {
    let request: Value = match serde_json::from_str(body) {
        Ok(value) => value,
        Err(error) => {
            return Some(error_response(
                &Value::Null,
                PARSE_ERROR,
                &format!("parse error: {error}"),
            ));
        }
    };

    let Some(object) = request.as_object() else {
        return Some(error_response(
            &Value::Null,
            INVALID_REQUEST,
            "request must be a JSON-RPC object",
        ));
    };

    let id = object.get("id").cloned();
    let params = object.get("params").cloned().unwrap_or(Value::Null);
    let method = object.get("method").and_then(Value::as_str);

    let Some(method) = method else {
        return id.map(|id| error_response(&id, INVALID_REQUEST, "missing `method`"));
    };

    // A message with no `id` member is a notification: it never gets a reply.
    let id = id?;

    Some(match dispatch(server, method, &params) {
        Ok(result) => success_response(&id, &result),
        Err(error) => error_response(&id, error.code, &error.message),
    })
}

// Route a request method to the matching server capability.
fn dispatch(server: &dyn McpServer, method: &str, params: &Value) -> Result<Value, McpError> {
    match method {
        "initialize" => Ok(initialize_result(server, params)),
        "ping" => Ok(json!({})),
        "tools/list" => Ok(json!({ "tools": server.tools() })),
        "tools/call" => tools_call(server, params),
        "resources/list" => Ok(json!({ "resources": server.resources() })),
        "resources/read" => resources_read(server, params),
        other => Err(McpError::method_not_found(format!("unknown method `{other}`"))),
    }
}

// Build the `initialize` result, echoing the client's protocol version when it
// sends one and advertising only the capabilities the server actually serves.
fn initialize_result(server: &dyn McpServer, params: &Value) -> Value {
    let protocol_version =
        params.get("protocolVersion").and_then(Value::as_str).unwrap_or(PROTOCOL_VERSION);

    let mut capabilities = serde_json::Map::new();
    if !server.tools().is_empty() {
        capabilities.insert("tools".to_owned(), json!({}));
    }
    if !server.resources().is_empty() {
        capabilities.insert("resources".to_owned(), json!({}));
    }

    json!({
        "protocolVersion": protocol_version,
        "capabilities": capabilities,
        "serverInfo": server.info(),
    })
}

fn tools_call(server: &dyn McpServer, params: &Value) -> Result<Value, McpError> {
    let name = params
        .get("name")
        .and_then(Value::as_str)
        .ok_or_else(|| McpError::invalid_params("missing tool `name`"))?;
    let arguments = params.get("arguments").cloned().unwrap_or_else(|| json!({}));
    Ok(json!(server.call_tool(name, &arguments)?))
}

fn resources_read(server: &dyn McpServer, params: &Value) -> Result<Value, McpError> {
    let uri = params
        .get("uri")
        .and_then(Value::as_str)
        .ok_or_else(|| McpError::invalid_params("missing resource `uri`"))?;
    Ok(json!({ "contents": [server.read_resource(uri)?] }))
}

fn success_response(id: &Value, result: &Value) -> String {
    json!({ "jsonrpc": "2.0", "id": id, "result": result }).to_string()
}

fn error_response(id: &Value, code: i32, message: &str) -> String {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": { "code": code, "message": message },
    })
    .to_string()
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};

    use super::super::types::METHOD_NOT_FOUND;
    use super::super::{
        CallToolResult, Implementation, McpError, Resource, ResourceContents, Tool,
    };
    use super::{McpServer, PROTOCOL_VERSION, handle_message};

    struct Docs;

    impl McpServer for Docs {
        fn info(&self) -> Implementation {
            Implementation::new("docs", "1.2.3")
        }

        fn tools(&self) -> Vec<Tool> {
            vec![Tool::new("read_doc", "read a document", json!({ "type": "object" }))]
        }

        fn call_tool(&self, name: &str, arguments: &Value) -> Result<CallToolResult, McpError> {
            if name != "read_doc" {
                return Err(McpError::method_not_found(format!("unknown tool `{name}`")));
            }
            match arguments.get("name").and_then(Value::as_str) {
                Some("guide") => Ok(CallToolResult::text("the guide body")),
                Some(other) => Ok(CallToolResult::error(format!("no such doc `{other}`"))),
                None => Err(McpError::invalid_params("missing `name`")),
            }
        }

        fn resources(&self) -> Vec<Resource> {
            vec![Resource::new("doc://guide", "guide", "the guide", "text/markdown")]
        }

        fn read_resource(&self, uri: &str) -> Result<ResourceContents, McpError> {
            if uri == "doc://guide" {
                Ok(ResourceContents::text(uri, "text/markdown", "the guide body"))
            } else {
                Err(McpError::invalid_params(format!("unknown resource `{uri}`")))
            }
        }
    }

    // Parse the `result` object of a successful reply to the given request.
    fn result_of(request: &Value) -> Value {
        let reply = handle_message(&Docs, &request.to_string()).expect("a request gets a reply");
        let value: Value = serde_json::from_str(&reply).expect("reply is JSON");
        assert_eq!(value["jsonrpc"], "2.0");
        assert_eq!(value["id"], request["id"]);
        value["result"].clone()
    }

    #[test]
    fn initialize() {
        let result = result_of(&json!({
            "jsonrpc": "2.0", "id": 1, "method": "initialize",
            "params": { "protocolVersion": "2025-03-26" }
        }));
        assert_eq!(result["protocolVersion"], "2025-03-26");
        assert!(result["capabilities"].get("tools").is_some());
        assert!(result["capabilities"].get("resources").is_some());
        assert_eq!(result["serverInfo"]["name"], "docs");
        assert_eq!(result["serverInfo"]["version"], "1.2.3");
    }

    #[test]
    fn initialize_default_version() {
        let result = result_of(&json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize" }));
        assert_eq!(result["protocolVersion"], PROTOCOL_VERSION);
    }

    #[test]
    fn ping() {
        let result = result_of(&json!({ "jsonrpc": "2.0", "id": 7, "method": "ping" }));
        assert_eq!(result, json!({}));
    }

    #[test]
    fn tools_list() {
        let result = result_of(&json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list" }));
        assert_eq!(result["tools"][0]["name"], "read_doc");
        assert_eq!(result["tools"][0]["inputSchema"]["type"], "object");
    }

    #[test]
    fn tools_call() {
        let result = result_of(&json!({
            "jsonrpc": "2.0", "id": 3, "method": "tools/call",
            "params": { "name": "read_doc", "arguments": { "name": "guide" } }
        }));
        assert_eq!(result["isError"], false);
        assert_eq!(result["content"][0]["type"], "text");
        assert_eq!(result["content"][0]["text"], "the guide body");
    }

    #[test]
    fn tools_call_error() {
        // A tool that runs but fails is a result with `isError`, not a JSON-RPC error.
        let result = result_of(&json!({
            "jsonrpc": "2.0", "id": 4, "method": "tools/call",
            "params": { "name": "read_doc", "arguments": { "name": "missing" } }
        }));
        assert_eq!(result["isError"], true);
    }

    #[test]
    fn resources_read() {
        let result = result_of(&json!({
            "jsonrpc": "2.0", "id": 5, "method": "resources/read",
            "params": { "uri": "doc://guide" }
        }));
        assert_eq!(result["contents"][0]["uri"], "doc://guide");
        assert_eq!(result["contents"][0]["mimeType"], "text/markdown");
        assert_eq!(result["contents"][0]["text"], "the guide body");
    }

    #[test]
    fn unknown_method() {
        let reply = handle_message(
            &Docs,
            &json!({ "jsonrpc": "2.0", "id": 9, "method": "nope" }).to_string(),
        )
        .expect("a request gets a reply");
        let value: Value = serde_json::from_str(&reply).expect("reply is JSON");
        assert_eq!(value["error"]["code"], METHOD_NOT_FOUND);
    }

    #[test]
    fn notification() {
        let reply =
            handle_message(&Docs, &json!({ "jsonrpc": "2.0", "method": "ping" }).to_string());
        assert!(reply.is_none(), "a message with no id is a notification");
    }

    #[test]
    fn malformed_json() {
        let reply = handle_message(&Docs, "{ not json").expect("a parse error still replies");
        let value: Value = serde_json::from_str(&reply).expect("reply is JSON");
        assert_eq!(value["error"]["code"], super::PARSE_ERROR);
        assert_eq!(value["id"], Value::Null);
    }
}
