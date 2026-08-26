//! Handler invocation and HTTP routing contracts.

use std::sync::atomic::{AtomicUsize, Ordering};

use axum::body::{Body, to_bytes};
use axum::response::{IntoResponse, Response};
use http::header::CONTENT_TYPE;
use http::{HeaderMap, HeaderValue, Method, Request, StatusCode};
use omnia_guest::api::http::{RawRequest, Router, get, get_with, post, post_with};
use omnia_guest::api::messaging::{
    Delivery, DeliveryError, Router as MessagingRouter, consume, consume_with,
};
use omnia_guest::api::{Client, Context, DecodeError, Handler, HttpError, Metadata};
use serde::{Deserialize, Serialize};
use tower::ServiceExt as _;

#[derive(Debug, Deserialize)]
struct EchoInput {
    name: String,
    count: Option<u32>,
}

#[derive(Debug, Serialize)]
struct EchoOutput {
    name: String,
    count: u32,
    owner: String,
    correlation_id: Option<String>,
}

impl<P: Send + Sync + 'static> Handler<P> for EchoInput {
    type Error = omnia_guest::Error;
    type Output = EchoOutput;

    fn handle(
        self, context: Context<'_, P>,
    ) -> impl std::future::Future<Output = Result<Self::Output, Self::Error>> + Send {
        std::future::ready(Ok(EchoOutput {
            name: self.name,
            count: self.count.unwrap_or(1),
            owner: context.owner.to_owned(),
            correlation_id: context.metadata.correlation_id.clone(),
        }))
    }
}

/// Input type promoted to a handler by `#[handler]` below.
#[derive(Debug, Deserialize)]
struct MacroEcho {
    name: String,
}

#[derive(Debug, Serialize)]
struct MacroEchoReply {
    name: String,
    owner: String,
}

#[omnia_guest::handler]
// The Handler contract is async; this test body just has nothing to await.
#[allow(clippy::unused_async)]
async fn macro_echo<P>(
    input: MacroEcho, context: Context<'_, P>,
) -> omnia_guest::Result<MacroEchoReply>
where
    P: Send + Sync + 'static,
{
    Ok(MacroEchoReply {
        name: input.name,
        owner: context.owner.to_owned(),
    })
}

struct StatefulProvider {
    calls: AtomicUsize,
}

#[derive(Serialize)]
struct ProviderObservation {
    address: usize,
    call: usize,
}

#[derive(Debug, Deserialize)]
struct ObserveInput {
    name: String,
}

impl Handler<StatefulProvider> for ObserveInput {
    type Error = omnia_guest::Error;
    type Output = ProviderObservation;

    fn handle(
        self, context: Context<'_, StatefulProvider>,
    ) -> impl std::future::Future<Output = Result<Self::Output, Self::Error>> + Send {
        let _ = self.name;
        std::future::ready(Ok(ProviderObservation {
            address: std::ptr::from_ref(context.provider).addr(),
            call: context.provider.calls.fetch_add(1, Ordering::SeqCst) + 1,
        }))
    }
}

fn router() -> axum::Router {
    Router::new(Client::new("test", ()))
        .route("/echo", get::<EchoInput, ()>())
        .route("/echo", post::<EchoInput, ()>())
        .route("/echo/{name}", get::<EchoInput, ()>())
        .route("/echo/{name}", post::<EchoInput, ()>())
        .into_axum()
}

async fn send(request: Request<Body>) -> (StatusCode, serde_json::Value) {
    let response = router().oneshot(request).await.expect("router serves request");
    let status = response.status();
    let bytes = to_bytes(response.into_body(), usize::MAX).await.expect("collect body");
    let value = serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null);
    (status, value)
}

#[tokio::test]
async fn invoke() {
    let client = Client::new("tenant", ());
    let metadata = Metadata {
        correlation_id: Some("call-1".to_string()),
        ..Metadata::default()
    };

    let output = client
        .call(
            EchoInput {
                name: "core".to_string(),
                count: None,
            },
            &metadata,
        )
        .await
        .expect("handler succeeds");

    assert_eq!(output.owner, "tenant");
    assert_eq!(output.correlation_id.as_deref(), Some("call-1"));
}

#[tokio::test]
async fn invoke_macro_handler() {
    let client = Client::new("tenant", ());

    let output = client
        .call(
            MacroEcho {
                name: "generated".to_string(),
            },
            &Metadata::default(),
        )
        .await
        .expect("handler succeeds");

    assert_eq!(output.name, "generated");
    assert_eq!(output.owner, "tenant");
}

#[tokio::test]
async fn get_query() {
    let request = Request::builder()
        .method(Method::GET)
        .uri("/echo?name=plan&count=3")
        .header("x-request-id", "request-1")
        .body(Body::empty())
        .expect("build request");
    let (status, value) = send(request).await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        value,
        serde_json::json!({
            "name": "plan",
            "count": 3,
            "owner": "test",
            "correlation_id": "request-1"
        })
    );
}

#[tokio::test]
async fn get_path_and_query() {
    let request = Request::builder()
        .method(Method::GET)
        .uri("/echo/slice?count=2")
        .body(Body::empty())
        .expect("build request");
    let (status, value) = send(request).await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(value["name"], "slice");
    assert_eq!(value["count"], 2);
}

#[tokio::test]
async fn get_missing_field() {
    let request = Request::builder()
        .method(Method::GET)
        .uri("/echo?count=3")
        .body(Body::empty())
        .expect("build request");
    let (status, _) = send(request).await;

    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn post_body_and_path() {
    let request = Request::builder()
        .method(Method::POST)
        .uri("/echo/slice")
        .header(http::header::CONTENT_TYPE, "application/json")
        .body(Body::from(r#"{"count":7}"#))
        .expect("build request");
    let (status, value) = send(request).await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(value["name"], "slice");
    assert_eq!(value["count"], 7);
}

#[tokio::test]
async fn post_empty_body() {
    let request = Request::builder()
        .method(Method::POST)
        .uri("/echo/slice")
        .body(Body::empty())
        .expect("build request");
    let (status, value) = send(request).await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(value["count"], 1);
}

#[tokio::test]
async fn post_non_object_body() {
    let request = Request::builder()
        .method(Method::POST)
        .uri("/echo/slice")
        .body(Body::from("[1,2]"))
        .expect("build request");
    let (status, _) = send(request).await;

    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn route_state_clones_share_provider() {
    let router = Router::new(Client::new(
        "test",
        StatefulProvider {
            calls: AtomicUsize::new(0),
        },
    ))
    .route("/first", get::<ObserveInput, StatefulProvider>())
    .route("/second", get::<ObserveInput, StatefulProvider>())
    .into_axum();

    let first = router
        .clone()
        .oneshot(
            Request::builder().uri("/first?name=first").body(Body::empty()).expect("build request"),
        )
        .await
        .expect("first route serves request");
    let second = router
        .oneshot(
            Request::builder()
                .uri("/second?name=second")
                .body(Body::empty())
                .expect("build request"),
        )
        .await
        .expect("second route serves request");
    let first: serde_json::Value = serde_json::from_slice(
        &to_bytes(first.into_body(), usize::MAX).await.expect("collect first body"),
    )
    .expect("decode first body");
    let second: serde_json::Value = serde_json::from_slice(
        &to_bytes(second.into_body(), usize::MAX).await.expect("collect second body"),
    )
    .expect("decode second body");

    assert_eq!(first["address"], second["address"]);
    assert_eq!(first["call"], 1);
    assert_eq!(second["call"], 2);
}

fn delivery(topic: Option<&str>, payload: &[u8]) -> Delivery {
    Delivery {
        topic: topic.map(str::to_owned),
        payload: payload.to_vec(),
        content_type: Some("application/json".to_string()),
        metadata: vec![("correlation-id".to_string(), "delivery-1".to_string())],
    }
}

#[tokio::test]
async fn messaging_exact_topic() {
    let router = MessagingRouter::new(Client::new("messages", ()))
        .route("events.created", consume::<EchoInput>());

    router
        .handle(delivery(Some("events.created"), br#"{"name":"message","count":2}"#))
        .await
        .expect("exact route handles delivery");
    assert_eq!(
        router.handle(delivery(Some("events.*"), br#"{"name":"message"}"#)).await,
        Err(DeliveryError::UnhandledTopic("events.*".to_string()))
    );
}

#[tokio::test]
async fn messaging_failures() {
    let router =
        MessagingRouter::new(Client::new("messages", ())).route("events", consume::<EchoInput>());

    assert_eq!(
        router.handle(delivery(None, br#"{"name":"message"}"#)).await,
        Err(DeliveryError::MissingTopic)
    );
    assert!(matches!(
        router.handle(delivery(Some("events"), b"not-json")).await,
        Err(DeliveryError::Rejected(_))
    ));
}

#[test]
#[should_panic(expected = "duplicate messaging topic")]
fn messaging_duplicate_topic() {
    let _router = MessagingRouter::new(Client::new("messages", ()))
        .route("events", consume::<EchoInput>())
        .route("events", consume::<EchoInput>());
}

#[derive(Debug)]
struct RawText {
    text: String,
}

impl<P: Send + Sync + 'static> Handler<P> for RawText {
    type Error = omnia_guest::Error;
    type Output = ();

    fn handle(
        self, _context: Context<'_, P>,
    ) -> impl std::future::Future<Output = Result<Self::Output, Self::Error>> + Send {
        let _ = self.text;
        std::future::ready(Ok(()))
    }
}

#[tokio::test]
async fn consume_with_raw_payload() {
    let router = MessagingRouter::new(Client::new("messages", ())).route(
        "events.raw",
        consume_with(|delivery: &Delivery| {
            std::str::from_utf8(&delivery.payload)
                .map(|text| RawText {
                    text: text.to_owned(),
                })
                .map_err(|error| DecodeError::new(format!("payload is not utf-8: {error}")))
        }),
    );

    router
        .handle(delivery(Some("events.raw"), b"not-json"))
        .await
        .expect("raw decoder handles a payload JSON consume rejects");
}

#[derive(Debug)]
struct JoinNames {
    names: Vec<String>,
}

impl<P: Send + Sync + 'static> Handler<P> for JoinNames {
    type Error = omnia_guest::Error;
    type Output = String;

    fn handle(
        self, _context: Context<'_, P>,
    ) -> impl std::future::Future<Output = Result<Self::Output, Self::Error>> + Send {
        std::future::ready(Ok(self.names.join(" & ")))
    }
}

#[derive(Debug)]
struct Greet {
    name: String,
    greeting: String,
}

impl<P: Send + Sync + 'static> Handler<P> for Greet {
    type Error = omnia_guest::Error;
    type Output = String;

    fn handle(
        self, _context: Context<'_, P>,
    ) -> impl std::future::Future<Output = Result<Self::Output, Self::Error>> + Send {
        std::future::ready(Ok(format!("{}, {}", self.greeting, self.name)))
    }
}

fn text_response(body: String) -> Response {
    (StatusCode::OK, [(CONTENT_TYPE, HeaderValue::from_static("text/plain; charset=utf-8"))], body)
        .into_response()
}

fn text_router() -> axum::Router {
    Router::new(Client::new("test", ()))
        .route(
            "/join",
            post_with(
                |raw: RawRequest<'_>| {
                    let body = std::str::from_utf8(raw.body)
                        .map_err(|error| DecodeError::new(format!("body is not utf-8: {error}")))?;
                    Ok(JoinNames {
                        names: body.lines().map(str::to_owned).collect(),
                    })
                },
                text_response,
            ),
        )
        .route(
            "/greet/{name}",
            get_with(
                |raw: RawRequest<'_>| {
                    let name = raw
                        .path_params
                        .iter()
                        .find(|(key, _)| key == "name")
                        .map(|(_, value)| value.clone())
                        .ok_or_else(|| DecodeError::new("missing name path parameter"))?;
                    let greeting = raw
                        .headers
                        .get("x-greeting")
                        .and_then(|value| value.to_str().ok())
                        .ok_or_else(|| DecodeError::new("missing x-greeting header"))?;
                    Ok(Greet {
                        name,
                        greeting: greeting.to_owned(),
                    })
                },
                text_response,
            ),
        )
        .into_axum()
}

async fn send_raw(
    router: axum::Router, request: Request<Body>,
) -> (StatusCode, Option<String>, String) {
    let response = router.oneshot(request).await.expect("router serves request");
    let status = response.status();
    let content_type = response
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned);
    let bytes = to_bytes(response.into_body(), usize::MAX).await.expect("collect body");
    (status, content_type, String::from_utf8_lossy(&bytes).into_owned())
}

#[tokio::test]
async fn post_with_text_codec() {
    let request = Request::builder()
        .method(Method::POST)
        .uri("/join")
        .body(Body::from("ada\ngrace"))
        .expect("build request");
    let (status, content_type, body) = send_raw(text_router(), request).await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(content_type.as_deref(), Some("text/plain; charset=utf-8"));
    assert_eq!(body, "ada & grace");
}

#[tokio::test]
async fn get_with_path_and_header() {
    let request = Request::builder()
        .method(Method::GET)
        .uri("/greet/ada")
        .header("x-greeting", "hello")
        .body(Body::empty())
        .expect("build request");
    let (status, content_type, body) = send_raw(text_router(), request).await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(content_type.as_deref(), Some("text/plain; charset=utf-8"));
    assert_eq!(body, "hello, ada");
}

#[tokio::test]
async fn get_with_missing_header() {
    let request = Request::builder()
        .method(Method::GET)
        .uri("/greet/ada")
        .body(Body::empty())
        .expect("build request");
    let (status, _, _) = send_raw(text_router(), request).await;

    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[derive(Debug)]
struct XmlGreet {
    name: String,
}

#[derive(Debug)]
struct EmptyName;

impl std::fmt::Display for EmptyName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("name cannot be empty")
    }
}

impl std::error::Error for EmptyName {}

impl From<EmptyName> for HttpError {
    fn from(error: EmptyName) -> Self {
        let body = format!("<error><code>empty_name</code><message>{error}</message></error>");
        Self::with_body(
            StatusCode::UNPROCESSABLE_ENTITY,
            HeaderValue::from_static("text/xml; charset=utf-8"),
            body.into_bytes(),
        )
    }
}

impl<P: Send + Sync + 'static> Handler<P> for XmlGreet {
    type Error = EmptyName;
    type Output = String;

    fn handle(
        self, _context: Context<'_, P>,
    ) -> impl std::future::Future<Output = Result<Self::Output, Self::Error>> + Send {
        std::future::ready(if self.name.is_empty() {
            Err(EmptyName)
        } else {
            Ok(format!("hello, {}", self.name))
        })
    }
}

fn parse_greet(headers: &HeaderMap, body: &[u8]) -> Result<XmlGreet, DecodeError> {
    let content_type = headers.get(CONTENT_TYPE).and_then(|value| value.to_str().ok());
    if content_type != Some("text/xml") {
        return Err(DecodeError::new("expected a text/xml request"));
    }
    let body = std::str::from_utf8(body)
        .map_err(|error| DecodeError::new(format!("body is not utf-8: {error}")))?;
    let start = body.find("<name>").map(|index| index + "<name>".len());
    let end = body.find("</name>");
    match (start, end) {
        (Some(start), Some(end)) if start <= end => Ok(XmlGreet {
            name: body[start..end].to_owned(),
        }),
        _ => Err(DecodeError::new("malformed greet document")),
    }
}

fn xml_router() -> axum::Router {
    Router::new(Client::new("xml", ()))
        .route(
            "/greet",
            post_with(
                |raw: RawRequest<'_>| parse_greet(raw.headers, raw.body),
                |greeting: String| {
                    (
                        StatusCode::OK,
                        [(CONTENT_TYPE, HeaderValue::from_static("text/xml; charset=utf-8"))],
                        format!("<greeting>{greeting}</greeting>"),
                    )
                        .into_response()
                },
            ),
        )
        .into_axum()
}

fn xml_request(body: &'static str) -> Request<Body> {
    Request::builder()
        .method(Method::POST)
        .uri("/greet")
        .header(CONTENT_TYPE, "text/xml")
        .body(Body::from(body))
        .expect("build request")
}

#[tokio::test]
async fn xml_success() {
    let (status, content_type, body) =
        send_raw(xml_router(), xml_request("<greet><name>ada</name></greet>")).await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(content_type.as_deref(), Some("text/xml; charset=utf-8"));
    assert_eq!(body, "<greeting>hello, ada</greeting>");
}

#[tokio::test]
async fn xml_handler_error() {
    let (status, content_type, body) =
        send_raw(xml_router(), xml_request("<greet><name></name></greet>")).await;

    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(content_type.as_deref(), Some("text/xml; charset=utf-8"));
    assert_eq!(
        body,
        "<error><code>empty_name</code><message>name cannot be empty</message></error>"
    );
}

#[tokio::test]
async fn xml_malformed_body() {
    let (status, content_type, _) = send_raw(xml_router(), xml_request("<greet>")).await;

    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(content_type.as_deref(), Some("text/plain; charset=utf-8"));
}
