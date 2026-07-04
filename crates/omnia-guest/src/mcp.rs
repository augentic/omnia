//! A stateless [Model Context Protocol][mcp] server for guests.
//!
//! A guest implements [`McpServer`] over its compiled-in capabilities and serves
//! [`router`] from its `wasi:http` `handle` export; the host's HTTP trigger is
//! the transport. Nothing here holds state between messages, so the host may
//! instantiate the guest fresh per request. Read-only is a property of the
//! implementation: a server exposes exactly the tools and resources it declares.
//!
//! [mcp]: https://modelcontextprotocol.io

mod protocol;
mod router;
mod types;

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
        Err(McpError::invalid_params(format!("unknown resource `{uri}`")))
    }
}
