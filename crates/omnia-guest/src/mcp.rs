//! A stateless [Model Context Protocol][mcp] server for guests.
//!
//! A guest implements [`McpServer`] over its compiled-in capabilities and serves
//! it from its `wasi:http` `handle` export via [`router`] and
//! [`omnia_wasi_http::serve`]; the host's HTTP trigger is the transport. Nothing
//! here holds state between
//! messages, so the host may instantiate the guest fresh per request. Read-only
//! is a property of the implementation: a server exposes exactly the tools and
//! resources it declares.
//!
//! [mcp]: https://modelcontextprotocol.io

mod protocol;
mod router;
mod types;

use serde::de::DeserializeOwned;
use serde_json::Value;

pub use self::protocol::{PROTOCOL_VERSION, handle_message};
pub use self::router::router;
pub use self::types::{
    CallToolResult, Content, Implementation, McpError, Resource, ResourceContents, Tool,
};

/// A stateless MCP server: implementors supply capabilities, the module handles
/// JSON-RPC framing and the Streamable HTTP transport.
pub trait McpServer: Send + Sync + 'static {
    /// Server identity reported in the `initialize` handshake.
    fn info(&self) -> Implementation;

    /// Usage instructions surfaced to clients in the `initialize` result.
    /// Defaults to none.
    fn instructions(&self) -> Option<String> {
        None
    }

    /// The tools advertised through `tools/list`.
    fn tools(&self) -> Vec<Tool>;

    /// Invoke the tool named `name` with its `arguments` object.
    ///
    /// # Errors
    ///
    /// Returns [`McpError`] for a protocol-level failure such as an unknown tool
    /// or malformed arguments. A tool that runs but fails should instead return
    /// `Ok(CallToolResult::error(..))` so the model sees the failure.
    fn call_tool(&self, name: &str, arguments: &Value) -> Result<CallToolResult, McpError>;

    /// The resources advertised through `resources/list`. Defaults to none.
    fn resources(&self) -> Vec<Resource> {
        Vec::new()
    }

    /// Read the resource identified by `uri`.
    ///
    /// # Errors
    ///
    /// Returns [`McpError`] when `uri` names no resource this server serves.
    fn read_resource(&self, uri: &str) -> Result<ResourceContents, McpError> {
        Err(McpError::resource_not_found(uri))
    }
}

/// Deserialize a tool's `arguments` object into `T`.
///
/// # Errors
///
/// Returns an invalid-params [`McpError`] when the arguments do not match `T`.
pub fn arguments<T: DeserializeOwned>(value: &Value) -> Result<T, McpError> {
    T::deserialize(value)
        .map_err(|error| McpError::invalid_params(format!("invalid arguments: {error}")))
}

#[cfg(test)]
mod tests {
    use serde::Deserialize;
    use serde_json::json;

    use super::arguments;

    #[derive(Debug, Deserialize)]
    struct ReadDoc {
        name: String,
    }

    #[test]
    fn arguments_deserialize() {
        let ReadDoc { name } = arguments(&json!({ "name": "guide" })).expect("valid arguments");
        assert_eq!(name, "guide");
    }

    #[test]
    fn arguments_invalid() {
        let error = arguments::<ReadDoc>(&json!({})).expect_err("missing field");
        assert_eq!(error.code, super::types::INVALID_PARAMS);
        assert!(error.message.contains("invalid arguments"), "{}", error.message);
    }
}
