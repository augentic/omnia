//! Stateless MCP JSON-RPC dispatch, independent of any transport.

use serde_json::{Value, json};

use super::McpServer;
use super::types::{INVALID_REQUEST, McpError, PARSE_ERROR};

/// The MCP protocol revision advertised when a client requests none — the
/// newest revision this server implements.
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

// revisions this stateless server behaves identically across; any other
// negotiates down to `PROTOCOL_VERSION`, as the spec allows
const SUPPORTED_VERSIONS: &[&str] = &["2025-06-18", "2025-03-26"];

// Capabilities advertise feature support, not list contents: the trait
// always answers `tools/list` and `resources/list`.
fn initialize_result(server: &dyn McpServer, params: &Value) -> Value {
    let protocol_version = params
        .get("protocolVersion")
        .and_then(Value::as_str)
        .filter(|version| SUPPORTED_VERSIONS.contains(version))
        .unwrap_or(PROTOCOL_VERSION);

    let mut result = json!({
        "protocolVersion": protocol_version,
        "capabilities": { "tools": {}, "resources": {} },
        "serverInfo": server.info(),
    });
    if let Some(instructions) = server.instructions() {
        result["instructions"] = Value::String(instructions);
    }
    result
}

fn tools_call(server: &dyn McpServer, params: &Value) -> Result<Value, McpError> {
    let name = params
        .get("name")
        .and_then(Value::as_str)
        .ok_or_else(|| McpError::invalid_params("missing tool `name`"))?;
    let arguments = params.get("arguments").cloned().unwrap_or_else(|| json!({}));
    let result = server.call_tool(name, &arguments)?;
    Ok(serde_json::to_value(result).expect("CallToolResult serializes"))
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
