//! Handler invocation and HTTP handler support contracts.

#![cfg(feature = "http")]

use std::future::{Ready, ready};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use axum::Json;
use axum::body::{Body, to_bytes};
use axum::extract::{Path, Query};
use axum::middleware::from_fn;
use axum::routing::get;
use http::header::CONTENT_TYPE;
use http::{Request, StatusCode};
use omnia_sdk::api::http::trace;
use omnia_sdk::api::messaging::Delivery;
use omnia_sdk::api::{Client, Context, ErrorBody, Format, Handlers, Metadata, Render, Router};
use omnia_sdk::not_found;
use serde::{Deserialize, Serialize};
use tower::ServiceExt as _;
use tracing::field::{Field, Visit};
use tracing::span::{Attributes, Id};
use tracing_subscriber::Registry;
use tracing_subscriber::layer::{Context as LayerContext, Layer, SubscriberExt as _};

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

type Span = (String, Vec<(String, String)>);

#[derive(Default)]
struct Spans(Arc<Mutex<Vec<Span>>>);

impl<S: tracing::Subscriber> Layer<S> for Spans {
    fn on_new_span(&self, attrs: &Attributes<'_>, _id: &Id, _ctx: LayerContext<'_, S>) {
        struct Fields(Vec<(String, String)>);

        impl Visit for Fields {
            fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
                self.0.push((field.name().to_owned(), format!("{value:?}")));
            }

            fn record_str(&mut self, field: &Field, value: &str) {
                self.0.push((field.name().to_owned(), value.to_owned()));
            }
        }

        let mut fields = Fields(Vec::new());
        attrs.record(&mut fields);
        self.0.lock().unwrap().push((attrs.metadata().name().to_owned(), fields.0));
    }
}

struct StatefulProvider {
    calls: AtomicUsize,
}

#[derive(Serialize)]
struct ProviderObservation {
    address: usize,
    call: usize,
}

#[derive(Debug, Serialize)]
struct Summary {
    name: String,
    count: u32,
}

fn echo<P: Send + Sync + 'static>(
    input: EchoInput, context: Context<P>,
) -> Ready<Result<EchoOutput, omnia_sdk::Error>> {
    ready(Ok(EchoOutput {
        name: input.name,
        count: input.count.unwrap_or(1),
        request_id: context.metadata.request_id,
        correlation_id: context.metadata.correlation_id,
        causation_id: context.metadata.causation_id,
        tenant: context.metadata.tenant,
    }))
}

async fn echo_route<P: Send + Sync + 'static>(
    context: Context<P>, Query(input): Query<EchoInput>,
) -> Result<Json<EchoOutput>, omnia_sdk::Error> {
    echo(input, context).await.map(Json)
}

fn handlers<P: Send + Sync + 'static>() -> Handlers<P> {
    Handlers::from([Handlers::new().route("/echo", get(echo_route)).topic("echo", echo)])
}

fn router() -> Router {
    handlers().with_state(Client::new(())).layer(from_fn(trace))
}

async fn send(
    router: impl Into<axum::Router>, request: Request<Body>,
) -> (StatusCode, serde_json::Value) {
    let response = router.into().oneshot(request).await.expect("router serves request");
    let status = response.status();
    let bytes = to_bytes(response.into_body(), usize::MAX).await.expect("collect body");
    let value = serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null);
    (status, value)
}

async fn send_raw(
    router: axum::Router, request: Request<Body>,
) -> (StatusCode, Option<String>, String) {
    let response = router.oneshot(request).await.expect("router serves request");
    let status = response.status();
    let content_type =
        response.headers().get(CONTENT_TYPE).map(|value| value.to_str().unwrap().to_owned());
    let bytes = to_bytes(response.into_body(), usize::MAX).await.expect("collect body");
    (status, content_type, String::from_utf8(bytes.to_vec()).expect("utf-8 body"))
}

#[tokio::test]
async fn invoke() {
    let client = Client::new(());
    let metadata = Metadata {
        correlation_id: Some("call-1".to_string()),
        tenant: Some("acme".to_string()),
        ..Metadata::default()
    };

    let output = client
        .call(
            echo,
            EchoInput {
                name: "core".to_string(),
                count: None,
            },
            &metadata,
        )
        .await
        .expect("handler succeeds");

    assert_eq!(output.correlation_id.as_deref(), Some("call-1"));
    assert_eq!(output.tenant.as_deref(), Some("acme"));
}

#[tokio::test]
async fn invoke_direct() {
    let context = Context::new((), Metadata::default());

    let output = echo(
        EchoInput {
            name: "direct".to_string(),
            count: Some(2),
        },
        context,
    )
    .await
    .expect("handler succeeds");

    assert_eq!(output.name, "direct");
    assert_eq!(output.count, 2);
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
    let router = axum::Router::new().route("/echo", get(echo_route)).with_state(Client::new(()));
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

async fn ids(metadata: Metadata, context: Context<()>) -> Json<Ids> {
    Json(Ids {
        metadata: metadata.request_id,
        context: context.metadata.request_id,
    })
}

// A request without an id gets one minted id, which the span, the `Metadata`
// extractor, and the `Context` extractor all carry.
#[tokio::test]
async fn missing_request_id() {
    let router = axum::Router::new()
        .route("/ids", get(ids))
        .layer(from_fn(trace))
        .with_state(Client::new(()));
    let request = Request::get("/ids").body(Body::empty()).expect("build request");
    let (status, value) = send(router, request).await;

    assert_eq!(status, StatusCode::OK);
    let minted = value["metadata"].as_str().expect("request id is minted");
    assert!(is_minted_id(minted), "{minted:?} is not 32 lowercase hex chars");
    assert_eq!(value["context"], minted);
}

#[tokio::test]
async fn trace_span() {
    let spans = Spans::default();
    let recorded = Arc::clone(&spans.0);
    let _guard = tracing::subscriber::set_default(Registry::default().with(spans));
    let request = Request::get("/echo?name=plan")
        .header("x-request-id", "req-1")
        .header("x-correlation-id", "corr-1")
        .body(Body::empty())
        .expect("build request");

    let (status, _) = send(router(), request).await;

    assert_eq!(status, StatusCode::OK);
    let (_, fields) = recorded
        .lock()
        .unwrap()
        .iter()
        .find(|(name, _)| name == "handler")
        .cloned()
        .expect("the trace layer opens a `handler` span");
    assert_eq!(
        fields,
        [
            ("request_id".to_owned(), "req-1".to_owned()),
            ("correlation_id".to_owned(), "corr-1".to_owned()),
        ]
    );
}

async fn observe(context: Context<StatefulProvider>) -> Json<ProviderObservation> {
    Json(ProviderObservation {
        address: std::ptr::from_ref(context.provider()).addr(),
        call: context.provider().calls.fetch_add(1, Ordering::SeqCst) + 1,
    })
}

#[tokio::test]
async fn context_shares_provider() {
    let router = axum::Router::new()
        .route("/first", get(observe))
        .route("/second", get(observe))
        .with_state(Client::new(StatefulProvider {
            calls: AtomicUsize::new(0),
        }));

    let (_, first) =
        send(router.clone(), Request::get("/first").body(Body::empty()).expect("build request"))
            .await;
    let (_, second) =
        send(router, Request::get("/second").body(Body::empty()).expect("build request")).await;

    assert_eq!(first["address"], second["address"]);
    assert_eq!(first["call"], 1);
    assert_eq!(second["call"], 2);
}

async fn lookup(Path(id): Path<String>) -> Result<Json<String>, omnia_sdk::Error> {
    Err(not_found!("no item {id}"))
}

#[tokio::test]
async fn error_body_json() {
    let router = axum::Router::new().route("/items/{id}", get(lookup)).with_state(Client::new(()));
    let request = Request::get("/items/42").body(Body::empty()).expect("build request");
    let (status, content_type, body) = send_raw(router, request).await;

    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(content_type.as_deref(), Some("application/json"));
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&body).expect("decode error body"),
        serde_json::json!({ "error": "not_found", "message": "no item 42" })
    );
}

async fn save() -> Result<Json<serde_json::Value>, omnia_sdk::Error> {
    Err(anyhow::anyhow!("disk full").context("could not save"))?;
    Ok(Json(serde_json::json!({ "saved": true })))
}

#[tokio::test]
async fn anyhow_error_body() {
    let router = axum::Router::new().route("/save", get(save)).with_state(Client::new(()));
    let request = Request::get("/save").body(Body::empty()).expect("build request");
    let (status, content_type, body) = send_raw(router, request).await;

    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(content_type.as_deref(), Some("application/json"));
    let body: ErrorBody = serde_json::from_str(&body).expect("decode error body");
    assert_eq!(body.error, "server_error");
    assert!(body.message.contains("could not save"), "the context is kept: {}", body.message);
    assert!(body.message.contains("disk full"), "the cause is kept: {}", body.message);
}

impl Render for Summary {
    fn render(&self, out: &mut dyn std::fmt::Write) -> std::fmt::Result {
        write!(out, "{} x{}", self.name, self.count)
    }
}

#[test]
fn format_encode() {
    let summary = Summary {
        name: "widget".to_string(),
        count: 3,
    };

    let text = Format::Text.encode(&summary);
    assert_eq!(text.media_type, "text/plain; charset=utf-8");
    assert_eq!(text.bytes, b"widget x3");

    let json = Format::Json.encode(&summary);
    assert_eq!(json.media_type, "application/json");
    let json = String::from_utf8(json.bytes).expect("json is utf-8");
    assert!(json.ends_with('\n'));
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&json).expect("decode json"),
        serde_json::json!({ "name": "widget", "count": 3 })
    );
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
        _ => None,
    });

    assert_eq!(metadata.request_id.as_deref(), Some("req-1"));
    assert_eq!(metadata.correlation_id.as_deref(), Some("corr-1"));
    assert_eq!(metadata.causation_id.as_deref(), Some("cause-1"));
    assert_eq!(metadata.tenant.as_deref(), Some("acme"));
    assert_eq!(metadata.deadline, None);
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

    let response = router.clone().oneshot(request).await.expect("router serves request");
    assert_eq!(response.status(), StatusCode::OK);

    let delivery = Delivery {
        topic: Some("echo".to_string()),
        payload: br#"{"name":"both"}"#.to_vec(),
        ..Delivery::default()
    };
    router.handle(delivery).await.expect("router dispatches delivery");
}

#[tokio::test]
async fn capability_error_taxonomy() {
    use omnia_sdk::{plugins, vcs};

    async fn route(Path(kind): Path<String>) -> Result<Json<()>, omnia_sdk::Error> {
        match kind.as_str() {
            "vcs-missing" => Err(vcs::Error::NotFound("v1".into()))?,
            "vcs-access" => Err(vcs::Error::Access("denied".into()))?,
            "vcs-pending" => Err(vcs::Error::Pending(vec!["a.rs".into(), "b.md".into()]))?,
            "vcs-other" => Err(vcs::Error::Other("boom".into()))?,
            "plugin-refused" => Err(plugins::Error::Refused("r".into()))?,
            "plugin-unavailable" => Err(plugins::Error::Unavailable("u".into()))?,
            "plugin-internal" => Err(plugins::Error::Internal("x".into()))?,
            _ => Ok(Json(())),
        }
    }

    let router = axum::Router::new().route("/err/{kind}", get(route)).with_state(Client::new(()));

    for (path, expected_status, expected_code) in [
        ("/err/vcs-missing", StatusCode::NOT_FOUND, "not-found"),
        ("/err/vcs-access", StatusCode::BAD_GATEWAY, "access"),
        ("/err/vcs-pending", StatusCode::BAD_REQUEST, "pending"),
        ("/err/vcs-other", StatusCode::INTERNAL_SERVER_ERROR, "other"),
        ("/err/plugin-refused", StatusCode::BAD_REQUEST, "refused"),
        ("/err/plugin-unavailable", StatusCode::BAD_GATEWAY, "unavailable"),
        ("/err/plugin-internal", StatusCode::INTERNAL_SERVER_ERROR, "internal"),
    ] {
        let request = Request::get(path).body(Body::empty()).expect("build request");
        let (status, _, body) = send_raw(router.clone(), request).await;
        assert_eq!(status, expected_status, "{path}");
        let body: ErrorBody = serde_json::from_str(&body).expect("decode error body");
        assert_eq!(body.error, expected_code, "{path}");
    }
}

#[derive(Deserialize)]
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
        vec![omnia_sdk::mcp::Tool::new(
            "read_doc",
            "read a document",
            serde_json::json!({ "type": "object" }),
        )]
    }

    fn call_tool(
        &self, name: &str, arguments: &serde_json::Value,
    ) -> Result<omnia_sdk::mcp::CallToolResult, omnia_sdk::mcp::McpError> {
        if name != "read_doc" {
            return Err(omnia_sdk::mcp::McpError::unknown_tool(name));
        }
        let ReadDoc { name } = omnia_sdk::mcp::arguments(arguments)?;
        if name == "guide" {
            Ok(omnia_sdk::mcp::CallToolResult::text("the guide body"))
        } else {
            Ok(omnia_sdk::mcp::CallToolResult::error(format!("no such doc `{name}`")))
        }
    }

    fn resources(&self) -> Vec<omnia_sdk::mcp::Resource> {
        vec![omnia_sdk::mcp::Resource::new("doc://guide", "guide", "the guide", "text/markdown")]
    }

    fn read_resource(
        &self, uri: &str,
    ) -> Result<omnia_sdk::mcp::ResourceContents, omnia_sdk::mcp::McpError> {
        if uri == "doc://guide" {
            Ok(omnia_sdk::mcp::ResourceContents::text(uri, "text/markdown", "the guide body"))
        } else {
            Err(omnia_sdk::mcp::McpError::resource_not_found(uri))
        }
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
            let req = Request::builder()
                .method(Method::POST)
                .uri("/mcp")
                .header(CONTENT_TYPE, "application/json")
                .body(Body::from(body.to_string()))
                .expect("request");
            send(app, req).await
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
    assert_eq!(list["result"]["tools"][0]["name"], "read_doc");

    let (_, call) = post(serde_json::json!({
        "jsonrpc": "2.0", "id": 3, "method": "tools/call",
        "params": { "name": "read_doc", "arguments": { "name": "guide" } }
    }))
    .await;
    assert_eq!(call["result"]["isError"], false);
    assert_eq!(call["result"]["content"][0]["text"], "the guide body");

    let (_, tool_err) = post(serde_json::json!({
        "jsonrpc": "2.0", "id": 4, "method": "tools/call",
        "params": { "name": "read_doc", "arguments": { "name": "missing" } }
    }))
    .await;
    assert_eq!(tool_err["result"]["isError"], true);

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
    let (_, res) = post(serde_json::json!({
        "jsonrpc": "2.0", "id": 7, "method": "resources/read",
        "params": { "uri": "doc://guide" }
    }))
    .await;
    assert_eq!(res["result"]["contents"][0]["text"], "the guide body");

    let (notif_status, _) =
        post(serde_json::json!({ "jsonrpc": "2.0", "method": "notifications/initialized" })).await;
    assert_eq!(notif_status, StatusCode::ACCEPTED);

    let get_req = Request::get("/mcp").body(Body::empty()).expect("get");
    let (get_status, _) = send(app, get_req).await;
    assert_eq!(get_status, StatusCode::METHOD_NOT_ALLOWED);
}
