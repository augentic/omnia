//! Streamable HTTP transport binding for an [`McpServer`].

use std::sync::Arc;

use axum::Router;
use axum::http::{Method, StatusCode, header};
use axum::response::{IntoResponse, Response};

use super::McpServer;
use super::protocol::handle_message;

/// An axum router serving `server` as a stateless MCP Streamable HTTP endpoint.
///
/// It matches every path (so it works behind any host route prefix): a `POST`
/// carries one JSON-RPC message, a notification is answered with `202 Accepted`
/// and no body, and any other method gets `405 Method Not Allowed`.
pub fn router(server: impl McpServer) -> Router {
    let server: Arc<dyn McpServer> = Arc::new(server);
    Router::new().fallback(move |method: Method, body: String| {
        let server = Arc::clone(&server);
        async move { respond(server.as_ref(), &method, &body) }
    })
}

fn respond(server: &dyn McpServer, method: &Method, body: &str) -> Response {
    if *method != Method::POST {
        return (StatusCode::METHOD_NOT_ALLOWED, "MCP endpoint accepts POST only").into_response();
    }
    handle_message(server, body).map_or_else(
        || StatusCode::ACCEPTED.into_response(),
        |text| ([(header::CONTENT_TYPE, "application/json")], text).into_response(),
    )
}

#[cfg(test)]
mod tests {
    use axum::body::{Body, to_bytes};
    use axum::http::{Method, Request, StatusCode, header};
    use serde_json::{Value, json};
    use tower::ServiceExt as _;

    use super::super::{CallToolResult, Implementation, McpError, McpServer, Tool};
    use super::router;

    struct Echo;

    impl McpServer for Echo {
        fn info(&self) -> Implementation {
            Implementation::new("echo", "0.0.0")
        }

        fn tools(&self) -> Vec<Tool> {
            vec![Tool::new("echo", "echoes text", json!({ "type": "object" }))]
        }

        fn call_tool(&self, name: &str, arguments: &Value) -> Result<CallToolResult, McpError> {
            if name != "echo" {
                return Err(McpError::unknown_tool(name));
            }
            let text = arguments.get("text").and_then(Value::as_str).unwrap_or_default();
            Ok(CallToolResult::text(text))
        }
    }

    // Drive one POST through the router, returning the status and body text.
    async fn post(body: &str) -> (StatusCode, String) {
        let request = Request::builder()
            .method(Method::POST)
            .uri("/mcp/docs")
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(body.to_owned()))
            .expect("build request");
        let response = router(Echo).oneshot(request).await.expect("router serves the request");
        let status = response.status();
        let bytes = to_bytes(response.into_body(), usize::MAX).await.expect("collect body");
        (status, String::from_utf8(bytes.to_vec()).expect("utf-8 body"))
    }

    #[tokio::test]
    async fn post_dispatch() {
        let request = json!({
            "jsonrpc": "2.0", "id": 1, "method": "tools/call",
            "params": { "name": "echo", "arguments": { "text": "hi" } }
        });
        let (status, body) = post(&request.to_string()).await;
        assert_eq!(status, StatusCode::OK);
        let value: Value = serde_json::from_str(&body).expect("json reply");
        assert_eq!(value["result"]["content"][0]["text"], "hi");
    }

    #[tokio::test]
    async fn notification() {
        let (status, body) =
            post(&json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }).to_string())
                .await;
        assert_eq!(status, StatusCode::ACCEPTED);
        assert!(body.is_empty(), "a notification gets no body");
    }

    #[tokio::test]
    async fn get_not_allowed() {
        let request = Request::builder()
            .method(Method::GET)
            .uri("/mcp/docs")
            .body(Body::empty())
            .expect("build request");
        let response = router(Echo).oneshot(request).await.expect("router serves the request");
        assert_eq!(response.status(), StatusCode::METHOD_NOT_ALLOWED);
    }
}
