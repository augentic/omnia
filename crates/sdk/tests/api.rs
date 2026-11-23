//! Handler invocation and HTTP handler support contracts.

#![cfg(feature = "http")]

mod support;

use std::collections::BTreeMap;
use std::future::{Ready, ready};
use std::time::{Duration, UNIX_EPOCH};

use axum::Json;
use axum::body::Body;
use axum::extract::{Path, Query};
use axum::middleware::from_fn;
use axum::routing::get;
use bytes::Bytes;
use http::header::CONTENT_TYPE;
use http::{Request, Response, StatusCode};
use http_body_util::BodyExt as _;
use omnia_sdk::http::trace;
use omnia_sdk::messaging::Delivery;
use omnia_sdk::{Context, ErrorBody, Handlers, Metadata, Router, not_found};
use serde::{Deserialize, Serialize};
use tower::ServiceExt as _;

use self::support::{Spans, fields};

#[derive(Debug, Deserialize)]
struct EchoInput {
    name: String,
    count: Option<u32>,
}

#[derive(Debug, Serialize)]
struct EchoOutput {
    name: String,
    count: u32,
    request_id: Option<String>,
    correlation_id: Option<String>,
    causation_id: Option<String>,
    tenant: Option<String>,
}

#[derive(Serialize)]
struct Ids {
    metadata: Option<String>,
    context: Option<String>,
}

fn echo(context: Context, input: EchoInput) -> Ready<Result<EchoOutput, omnia_sdk::Error>> {
    ready(Ok(EchoOutput {
        name: input.name,
        count: input.count.unwrap_or(1),
        request_id: context.metadata.request_id,
        correlation_id: context.metadata.correlation_id,
        causation_id: context.metadata.causation_id,
        tenant: context.metadata.tenant,
    }))
}

async fn echo_route(
    context: Context, Query(input): Query<EchoInput>,
) -> Result<Json<EchoOutput>, omnia_sdk::Error> {
    echo(context, input).await.map(Json)
}

fn handlers() -> Handlers {
    Handlers::from([Handlers::new().route("/echo", get(echo_route)).topic("echo", echo)])
}

fn router() -> Router {
    handlers().into_router().layer(from_fn(trace))
}

// named only by the `export!` macros, so a native build that drops the name
// fails `-D warnings` with `dead_code`
fn exported() -> Router {
    handlers().into_router()
}

omnia_sdk::http::export!(exported);
omnia_sdk::messaging::export!(exported);
omnia_sdk::command::export!(exported);

async fn respond(router: impl Into<axum::Router>, request: Request<Body>) -> Response<Bytes> {
    let response = router.into().oneshot(request).await.expect("an axum router is infallible");
    let (parts, body) = response.into_parts();
    let body = body.collect().await.expect("the response body is whole").to_bytes();
    Response::from_parts(parts, body)
}

async fn send(
    router: impl Into<axum::Router>, request: Request<Body>,
) -> (StatusCode, serde_json::Value) {
    let response = respond(router, request).await;
    let value = serde_json::from_slice(response.body()).unwrap_or(serde_json::Value::Null);
    (response.status(), value)
}

async fn send_raw(
    router: axum::Router, request: Request<Body>,
) -> (StatusCode, Option<String>, String) {
    let response = respond(router, request).await;
    let content_type =
        response.headers().get(CONTENT_TYPE).map(|value| value.to_str().unwrap().to_owned());
    let body = String::from_utf8(response.body().to_vec()).expect("utf-8 body");
    (response.status(), content_type, body)
}

#[tokio::test]
async fn context_extractor() {
    let request = Request::get("/echo?name=plan&count=3")
        .header("x-request-id", "req-1")
        .header("x-correlation-id", "corr-1")
        .header("x-causation-id", "cause-1")
        .header("x-tenant", "acme")
        .body(Body::empty())
        .expect("build request");
    let (status, value) = send(router(), request).await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(value["name"], "plan");
    assert_eq!(value["count"], 3);
    assert_eq!(value["request_id"], "req-1");
    assert_eq!(value["correlation_id"], "corr-1");
    assert_eq!(value["causation_id"], "cause-1");
    assert_eq!(value["tenant"], "acme");
}

// The extractor stands alone: without the `trace` layer it reads the headers itself.
#[tokio::test]
async fn extractor_no_trace() {
    let router = axum::Router::new().route("/echo", get(echo_route));
    let request = Request::get("/echo?name=plan")
        .header("x-request-id", "req-1")
        .body(Body::empty())
        .expect("build request");
    let (status, value) = send(router, request).await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(value["request_id"], "req-1");
    assert_eq!(value["correlation_id"], "req-1");
}

fn is_minted_id(id: &str) -> bool {
    id.len() == 32 && id.bytes().all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
}

async fn ids(metadata: Metadata, context: Context) -> Json<Ids> {
    Json(Ids {
        metadata: metadata.request_id,
        context: context.metadata.request_id,
    })
}

// A request without an id gets one minted id, which the span, the `Metadata`
// extractor, and the `Context` extractor all carry.
#[tokio::test]
async fn missing_request_id() {
    let router = axum::Router::new().route("/ids", get(ids)).layer(from_fn(trace));
    let request = Request::get("/ids").body(Body::empty()).expect("build request");
    let (status, value) = send(router, request).await;

    assert_eq!(status, StatusCode::OK);
    let minted = value["metadata"].as_str().expect("request id is minted");
    assert!(is_minted_id(minted), "{minted:?} is not 32 lowercase hex chars");
    assert_eq!(value["context"], minted);
}

#[tokio::test]
async fn trace_span() {
    let (spans, _guard) = Spans::install();
    let request = Request::get("/echo?name=plan")
        .header("x-request-id", "req-1")
        .header("x-correlation-id", "corr-1")
        .body(Body::empty())
        .expect("build request");

    let (status, _) = send(router(), request).await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        spans.handler(),
        fields([
            ("request_id", "req-1"),
            ("correlation_id", "corr-1"),
            ("method", "GET"),
            ("route", "/echo"),
        ])
    );
}

#[tokio::test]
async fn trace_route_template() {
    let (spans, _guard) = Spans::install();
    let router = axum::Router::new().route("/items/{id}", get(lookup)).layer(from_fn(trace));
    let request = Request::get("/items/42").body(Body::empty()).expect("build request");

    let (status, _) = send(router, request).await;

    assert_eq!(status, StatusCode::NOT_FOUND);
    assert!(spans.handler().contains(&("route".to_owned(), "/items/{id}".to_owned())));
}

#[tokio::test]
async fn trace_unmatched_path() {
    let (spans, _guard) = Spans::install();
    let request = Request::get("/missing")
        .header("x-request-id", "req-1")
        .header("x-correlation-id", "corr-1")
        .body(Body::empty())
        .expect("build request");

    let (status, _) = send(router(), request).await;

    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(
        spans.handler(),
        fields([("request_id", "req-1"), ("correlation_id", "corr-1"), ("method", "GET")])
    );
}

async fn lookup(Path(id): Path<String>) -> Result<Json<String>, omnia_sdk::Error> {
    Err(not_found!("no item {id}"))
}

#[tokio::test]
async fn error_body_json() {
    let router = axum::Router::new().route("/items/{id}", get(lookup));
    let request = Request::get("/items/42").body(Body::empty()).expect("build request");
    let (status, content_type, body) = send_raw(router, request).await;

    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(content_type.as_deref(), Some("application/json"));
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&body).expect("decode error body"),
        serde_json::json!({ "error": "not_found", "message": "no item 42" })
    );
}

#[test]
fn error_kinds() {
    use omnia_sdk::{Error, ErrorKind, bad_gateway, bad_request, conflict, server_error};

    for (error, kind, status, exit, code) in [
        (bad_request!("no name"), ErrorKind::BadRequest, StatusCode::BAD_REQUEST, 1, "bad_request"),
        (not_found!("no item"), ErrorKind::NotFound, StatusCode::NOT_FOUND, 2, "not_found"),
        (conflict!("taken"), ErrorKind::Conflict, StatusCode::CONFLICT, 5, "conflict"),
        (
            server_error!("full"),
            ErrorKind::ServerError,
            StatusCode::INTERNAL_SERVER_ERROR,
            3,
            "server_error",
        ),
        (bad_gateway!("silent"), ErrorKind::BadGateway, StatusCode::BAD_GATEWAY, 4, "bad_gateway"),
    ] {
        assert_eq!(error.kind(), kind, "{error}");
        assert_eq!(error.status(), status, "{error}");
        assert_eq!(error.exit_code(), exit, "{error}");
        assert_eq!(error.code(), code, "{error}");
    }

    let quota = Error::new(ErrorKind::Conflict, "quota", "3 of 3 spent");
    assert_eq!(quota, Error::conflict("quota", format!("{} of {} spent", 3, 3)));
    assert_eq!(quota.to_string(), "code: quota, description: 3 of 3 spent");
}

async fn save() -> Result<Json<serde_json::Value>, omnia_sdk::Error> {
    Err(anyhow::anyhow!("disk full").context("could not save"))?;
    Ok(Json(serde_json::json!({ "saved": true })))
}

#[tokio::test]
async fn anyhow_error_body() {
    let router = axum::Router::new().route("/save", get(save));
    let request = Request::get("/save").body(Body::empty()).expect("build request");
    let (status, content_type, body) = send_raw(router, request).await;

    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(content_type.as_deref(), Some("application/json"));
    let body: ErrorBody = serde_json::from_str(&body).expect("decode error body");
    assert_eq!(body.error, "server_error");
    assert!(body.message.contains("could not save"), "the context is kept: {}", body.message);
    assert!(body.message.contains("disk full"), "the cause is kept: {}", body.message);
}

#[test]
fn from_lookup_mints() {
    let first = Metadata::from_lookup(|_| None);
    let second = Metadata::from_lookup(|_| None);

    let request_id = first.request_id.as_deref().expect("request id is minted");
    assert!(is_minted_id(request_id));
    assert_eq!(first.correlation_id, first.request_id);
    assert_eq!(first.causation_id, None);
    assert_eq!(first.tenant, None);
    assert_eq!(first.deadline, None);
    assert_ne!(first.request_id, second.request_id);
    assert_eq!(Metadata::default().request_id, None);
}

#[test]
fn from_lookup_reads() {
    let metadata = Metadata::from_lookup(|name| match name {
        "request-id" => Some("req-1".to_owned()),
        "correlation-id" => Some("corr-1".to_owned()),
        "causation-id" => Some("cause-1".to_owned()),
        "tenant" => Some("acme".to_owned()),
        "deadline" => Some("1700000000123".to_owned()),
        _ => None,
    });

    assert_eq!(metadata.request_id.as_deref(), Some("req-1"));
    assert_eq!(metadata.correlation_id.as_deref(), Some("corr-1"));
    assert_eq!(metadata.causation_id.as_deref(), Some("cause-1"));
    assert_eq!(metadata.tenant.as_deref(), Some("acme"));
    assert_eq!(metadata.deadline, Some(UNIX_EPOCH + Duration::from_millis(1_700_000_000_123)));

    let unparseable = Metadata::from_lookup(|name| (name == "deadline").then(|| "soon".to_owned()));
    assert_eq!(unparseable.deadline, None);
}

#[test]
fn from_lookup_correlation() {
    let metadata = Metadata::from_lookup(|name| (name == "request-id").then(|| "req-2".to_owned()));

    assert_eq!(metadata.request_id.as_deref(), Some("req-2"));
    assert_eq!(metadata.correlation_id.as_deref(), Some("req-2"));
    assert_eq!(metadata.causation_id, None);
}

#[tokio::test]
async fn route_and_topic() {
    let router = router();
    let request = Request::get("/echo?name=both").body(Body::empty()).expect("build request");

    let response = respond(router.clone(), request).await;
    assert_eq!(response.status(), StatusCode::OK);

    router
        .handle(Delivery::new("echo", br#"{"name":"both"}"#))
        .await
        .expect("router dispatches delivery");
}

// Every interface shares the one `omnia:types` error, so one table covers
// every capability's failures. A typed outcome is the caller's to act on.
#[tokio::test]
async fn capability_errors() {
    use omnia_sdk::keyvalue::types::Error as Wire;
    use omnia_sdk::model::{Budget, Exhausted};
    use omnia_sdk::plugins::Conflict;
    use omnia_sdk::vcs::{Diverged, Occupied, Unmerged};

    async fn route(Path(kind): Path<String>) -> Result<Json<()>, omnia_sdk::Error> {
        let occupied = || Occupied {
            detail: "work".into(),
        };
        match kind.as_str() {
            "not-found" => Err(Wire::NotFound("no model `slow`".into()))?,
            "denied" => Err(Wire::Denied("no key".into()))?,
            "invalid" => Err(Wire::Invalid("no messages".into()))?,
            "unsupported" => Err(Wire::Unsupported("no effort".into()))?,
            "unavailable" => Err(Wire::Unavailable("rate limited".into()))?,
            "other" => Err(Wire::Other("bridge gone".into()))?,
            "exhausted" => Err(Exhausted {
                budget: Budget::Responses,
                detail: "3 rounds".into(),
                usage: None,
            })?,
            "conflict" => Err(Conflict {
                found: "sha256:ab".into(),
                current: None,
            })?,
            "occupied" => Err(occupied())?,
            "diverged" => Err(Diverged {
                current: Some("abc".into()),
            })?,
            "conflicts" => Err(Unmerged::Conflicts(vec!["Cargo.lock".into()]))?,
            "pending" => Err(Unmerged::Pending(vec!["a.rs".into(), "b.md".into()]))?,
            "wire-context" => {
                Err(anyhow::Error::from(Wire::Unavailable("store down".into())).context("read"))?
            }
            "outcome-context" => Err(anyhow::Error::from(occupied()).context("add"))?,
            _ => Ok(Json(())),
        }
    }

    let router = axum::Router::new().route("/err/{kind}", get(route));

    for (case, expected_status, expected_code, expected_message) in [
        ("not-found", StatusCode::INTERNAL_SERVER_ERROR, "not-found", "no model `slow`"),
        ("denied", StatusCode::INTERNAL_SERVER_ERROR, "denied", "no key"),
        ("invalid", StatusCode::INTERNAL_SERVER_ERROR, "invalid", "no messages"),
        ("unsupported", StatusCode::INTERNAL_SERVER_ERROR, "unsupported", "no effort"),
        ("unavailable", StatusCode::BAD_GATEWAY, "unavailable", "rate limited"),
        ("other", StatusCode::INTERNAL_SERVER_ERROR, "other", "bridge gone"),
        (
            "exhausted",
            StatusCode::BAD_REQUEST,
            "exhausted",
            "the responses budget is exhausted: 3 rounds",
        ),
        (
            "conflict",
            StatusCode::CONFLICT,
            "conflict",
            "the bytes hash to sha256:ab, not the pinned digest",
        ),
        ("occupied", StatusCode::CONFLICT, "occupied", "the place is occupied: work"),
        (
            "diverged",
            StatusCode::CONFLICT,
            "diverged",
            "the remote's bookmark holds commits the push does not, last seen at abc",
        ),
        ("conflicts", StatusCode::CONFLICT, "conflicts", "the merge left conflicts in Cargo.lock"),
        (
            "pending",
            StatusCode::CONFLICT,
            "pending",
            "the working copy holds pending changes in a.rs, b.md",
        ),
        ("wire-context", StatusCode::BAD_GATEWAY, "unavailable", "read: store down"),
        ("outcome-context", StatusCode::CONFLICT, "occupied", "add: the place is occupied: work"),
    ] {
        let request =
            Request::get(format!("/err/{case}")).body(Body::empty()).expect("build request");
        let (status, _, body) = send_raw(router.clone(), request).await;
        assert_eq!(status, expected_status, "{case}");
        let body: ErrorBody = serde_json::from_str(&body).expect("decode error body");
        assert_eq!(body.error, expected_code, "{case}");
        assert_eq!(body.message, expected_message, "{case}");
    }
}

// The wire form of a built request is asserted from a guest in the
// `wasi-http` suite. These are the parts no wire shows.
#[test]
fn bearer_token() {
    let request = omnia_sdk::http::get("https://origin.example/")
        .bearer("t0ken")
        .build()
        .expect("a valid request");

    let logged = format!("{:?}", request.headers());
    assert!(!logged.contains("t0ken"), "{logged}");
}

#[test]
fn explicit_content_type() {
    let request = omnia_sdk::http::patch("https://origin.example/orders/1")
        .header(CONTENT_TYPE, "application/merge-patch+json")
        .json(&serde_json::json!({ "quantity": 3 }))
        .build()
        .expect("a valid request");

    assert_eq!(request.headers()[CONTENT_TYPE], "application/merge-patch+json");
}

#[test]
fn unparseable_parts() {
    let origin = "https://origin.example/";
    for (case, builder) in [
        ("uri", omnia_sdk::http::get("not a uri")),
        ("header", omnia_sdk::http::get(origin).header("x-probe", "line\nbreak")),
        ("bearer", omnia_sdk::http::get(origin).bearer("line\nbreak")),
        ("json", omnia_sdk::http::post(origin).json(&BTreeMap::from([((1, 2), 3)]))),
    ] {
        let err = builder.build().expect_err(case);
        assert_eq!(err.status(), StatusCode::INTERNAL_SERVER_ERROR, "{case}");
        assert_eq!(err.code(), "invalid", "{case}");
    }
}

#[derive(Deserialize, schemars::JsonSchema)]
struct ReadDoc {
    name: String,
}

struct Docs;

impl omnia_sdk::mcp::McpServer for Docs {
    fn info(&self) -> omnia_sdk::mcp::Implementation {
        omnia_sdk::mcp::Implementation::new("docs", "1.2.3")
    }

    fn instructions(&self) -> Option<String> {
        Some("Read the guide first.".to_owned())
    }

    fn tools(&self) -> Vec<omnia_sdk::mcp::Tool> {
        vec![omnia_sdk::mcp::Tool::of::<ReadDoc>("read_doc", "read a document")]
    }

    fn call_tool(
        &self, name: &str, arguments: &serde_json::Value,
    ) -> impl Future<Output = Result<omnia_sdk::mcp::CallToolResult, omnia_sdk::mcp::McpError>> + Send
    {
        let result = if name == "read_doc" {
            omnia_sdk::mcp::arguments(arguments).map(|ReadDoc { name }| {
                if name == "guide" {
                    omnia_sdk::mcp::CallToolResult::text("the guide body")
                } else {
                    omnia_sdk::mcp::CallToolResult::error(format!("no such doc `{name}`"))
                }
            })
        } else {
            Err(omnia_sdk::mcp::McpError::unknown_tool(name))
        };
        ready(result)
    }

    fn resources(&self) -> Vec<omnia_sdk::mcp::Resource> {
        vec![omnia_sdk::mcp::Resource::new("doc://guide", "guide", "the guide", "text/markdown")]
    }

    fn read_resource(
        &self, uri: &str,
    ) -> impl Future<Output = Result<omnia_sdk::mcp::ResourceContents, omnia_sdk::mcp::McpError>> + Send
    {
        ready(if uri == "doc://guide" {
            Ok(omnia_sdk::mcp::ResourceContents::text(uri, "text/markdown", "the guide body"))
        } else {
            Err(omnia_sdk::mcp::McpError::resource_not_found(uri))
        })
    }
}

#[tokio::test]
async fn mcp_router() {
    use http::Method;
    use omnia_sdk::mcp::{self, PROTOCOL_VERSION};

    let app = mcp::router(Docs);
    let post = |body: serde_json::Value| {
        let app = app.clone();
        async move {
            let request = Request::builder()
                .method(Method::POST)
                .uri("/mcp")
                .header(CONTENT_TYPE, "application/json")
                .body(Body::from(body.to_string()))
                .expect("request");
            send(app, request).await
        }
    };

    // initialize (default version and negotiated version)
    let (status, init) =
        post(serde_json::json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize" })).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(init["result"]["protocolVersion"], PROTOCOL_VERSION);
    assert_eq!(init["result"]["serverInfo"]["name"], "docs");
    assert_eq!(init["result"]["instructions"], "Read the guide first.");

    // tools/list and tools/call (valid, tool error, invalid params, unknown tool)
    let (_, list) =
        post(serde_json::json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list" })).await;
    let tool = &list["result"]["tools"][0];
    assert_eq!(tool["name"], "read_doc");
    assert_eq!(tool["inputSchema"]["properties"]["name"]["type"], "string");
    assert_eq!(tool["inputSchema"]["required"], serde_json::json!(["name"]));

    let (_, call) = post(serde_json::json!({
        "jsonrpc": "2.0", "id": 3, "method": "tools/call",
        "params": { "name": "read_doc", "arguments": { "name": "guide" } }
    }))
    .await;
    assert_eq!(call["result"]["isError"], false);
    assert_eq!(call["result"]["content"][0]["text"], "the guide body");

    let (_, tool_error) = post(serde_json::json!({
        "jsonrpc": "2.0", "id": 4, "method": "tools/call",
        "params": { "name": "read_doc", "arguments": { "name": "missing" } }
    }))
    .await;
    assert_eq!(tool_error["result"]["isError"], true);

    let (_, bad_args) = post(serde_json::json!({
        "jsonrpc": "2.0", "id": 5, "method": "tools/call",
        "params": { "name": "read_doc", "arguments": {} }
    }))
    .await;
    assert!(bad_args["error"]["message"].as_str().unwrap().contains("invalid arguments"));

    let (_, unknown_tool) = post(serde_json::json!({
        "jsonrpc": "2.0", "id": 6, "method": "tools/call",
        "params": { "name": "nope" }
    }))
    .await;
    assert_eq!(unknown_tool["error"]["message"], "Unknown tool: nope");

    // resources/read and notification (202 Accepted) and GET (405 Method Not Allowed)
    let (_, resource) = post(serde_json::json!({
        "jsonrpc": "2.0", "id": 7, "method": "resources/read",
        "params": { "uri": "doc://guide" }
    }))
    .await;
    assert_eq!(resource["result"]["contents"][0]["text"], "the guide body");

    let (notified, _) =
        post(serde_json::json!({ "jsonrpc": "2.0", "method": "notifications/initialized" })).await;
    assert_eq!(notified, StatusCode::ACCEPTED);

    let request = Request::get("/mcp").body(Body::empty()).expect("get");
    let (get_status, _) = send(app, request).await;
    assert_eq!(get_status, StatusCode::METHOD_NOT_ALLOWED);
}
