//! Model Context Protocol schema types, mirroring the MCP wire format.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// JSON-RPC code for a request that was not valid JSON.
pub(super) const PARSE_ERROR: i32 = -32700;
/// JSON-RPC code for a well-formed but invalid request.
pub(super) const INVALID_REQUEST: i32 = -32600;
/// JSON-RPC code for an unknown method.
pub(super) const METHOD_NOT_FOUND: i32 = -32601;
/// JSON-RPC code for invalid method parameters.
pub(super) const INVALID_PARAMS: i32 = -32602;

/// Server identity reported in the `initialize` response (`serverInfo`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Implementation {
    /// Server name.
    pub name: String,
    /// Server version string.
    pub version: String,
}

impl Implementation {
    /// Server identity from a name and version.
    #[must_use]
    pub fn new(name: impl Into<String>, version: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            version: version.into(),
        }
    }
}

/// A tool advertised through `tools/list` and invoked through `tools/call`.
// `input_schema` is a `serde_json::Value`, which is not `Eq` (it carries f64),
// so this type can only be `PartialEq`.
#[allow(clippy::derive_partial_eq_without_eq)]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Tool {
    /// Unique tool name.
    pub name: String,
    /// Human-readable description shown to the model.
    pub description: String,
    /// JSON Schema for the tool's `arguments` object.
    pub input_schema: Value,
}

impl Tool {
    /// A tool from a name, description, and JSON Schema for its arguments.
    #[must_use]
    pub fn new(
        name: impl Into<String>, description: impl Into<String>, input_schema: Value,
    ) -> Self {
        Self {
            name: name.into(),
            description: description.into(),
            input_schema,
        }
    }
}

/// A single content block in a tool result or resource read.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum Content {
    /// A UTF-8 text block.
    Text {
        /// The text payload.
        text: String,
    },
}

impl Content {
    /// A text content block.
    #[must_use]
    pub fn text(text: impl Into<String>) -> Self {
        Self::Text { text: text.into() }
    }
}

/// The result of a `tools/call`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CallToolResult {
    /// Ordered content blocks returned to the model.
    pub content: Vec<Content>,
    /// Whether the content represents a tool-level error.
    pub is_error: bool,
}

impl CallToolResult {
    /// A successful result carrying a single text block.
    #[must_use]
    pub fn text(text: impl Into<String>) -> Self {
        Self {
            content: vec![Content::text(text)],
            is_error: false,
        }
    }

    /// A tool-level error carrying an explanatory text block.
    #[must_use]
    pub fn error(text: impl Into<String>) -> Self {
        Self {
            content: vec![Content::text(text)],
            is_error: true,
        }
    }
}

/// A resource advertised through `resources/list`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Resource {
    /// Resource URI, the key passed back to `resources/read`.
    pub uri: String,
    /// Short display name.
    pub name: String,
    /// Human-readable description.
    pub description: String,
    /// MIME type of the resource body.
    pub mime_type: String,
}

impl Resource {
    /// A resource descriptor.
    #[must_use]
    pub fn new(
        uri: impl Into<String>, name: impl Into<String>, description: impl Into<String>,
        mime_type: impl Into<String>,
    ) -> Self {
        Self {
            uri: uri.into(),
            name: name.into(),
            description: description.into(),
            mime_type: mime_type.into(),
        }
    }
}

/// The body returned by `resources/read`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResourceContents {
    /// URI of the resource that was read.
    pub uri: String,
    /// MIME type of `text`.
    pub mime_type: String,
    /// The resource body as UTF-8 text.
    pub text: String,
}

impl ResourceContents {
    /// A text resource body.
    #[must_use]
    pub fn text(
        uri: impl Into<String>, mime_type: impl Into<String>, text: impl Into<String>,
    ) -> Self {
        Self {
            uri: uri.into(),
            mime_type: mime_type.into(),
            text: text.into(),
        }
    }
}

/// A JSON-RPC error returned for a protocol-level failure.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct McpError {
    /// JSON-RPC error code.
    pub code: i32,
    /// Human-readable error message.
    pub message: String,
}

impl McpError {
    /// A method-not-found error (`-32601`).
    #[must_use]
    pub fn method_not_found(message: impl Into<String>) -> Self {
        Self {
            code: METHOD_NOT_FOUND,
            message: message.into(),
        }
    }

    /// An invalid-params error (`-32602`).
    #[must_use]
    pub fn invalid_params(message: impl Into<String>) -> Self {
        Self {
            code: INVALID_PARAMS,
            message: message.into(),
        }
    }
}
