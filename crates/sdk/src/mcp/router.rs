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
