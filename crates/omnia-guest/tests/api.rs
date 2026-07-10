//! Operation invocation and HTTP routing contracts.

use std::any::TypeId;
use std::sync::atomic::{AtomicUsize, Ordering};

use axum::body::{Body, to_bytes};
use axum::response::{IntoResponse, Response};
use http::{Method, Request, StatusCode};
use omnia_guest::api::http::{Projector, Router, get, get_with, post};
use omnia_guest::api::messaging::{
    Delivery, DeliveryError, Outcome as DeliveryOutcome, Projector as DeliveryProjector,
    Router as MessagingRouter, consume,
};
use omnia_guest::api::{CallContext, Invocation, Invoker, Metadata, Operation, Provider};
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

struct Echo;

impl<P: Provider> Operation<P> for Echo {
    type Error = omnia_guest::Error;
    type Input = EchoInput;
    type Output = EchoOutput;

    async fn call(
        input: Self::Input, context: CallContext<'_, P>,
    ) -> Result<Self::Output, Self::Error> {
        Ok(EchoOutput {
            name: input.name,
            count: input.count.unwrap_or(1),
            owner: context.owner.to_owned(),
            correlation_id: context.metadata.correlation_id.clone(),
        })
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

struct ObserveProvider;

impl Operation<StatefulProvider> for ObserveProvider {
    type Error = omnia_guest::Error;
    type Input = EchoInput;
    type Output = ProviderObservation;

    async fn call(
        _input: Self::Input, context: CallContext<'_, StatefulProvider>,
    ) -> Result<Self::Output, Self::Error> {
        Ok(ProviderObservation {
            address: std::ptr::from_ref(context.provider).addr(),
            call: context.provider.calls.fetch_add(1, Ordering::SeqCst) + 1,
        })
    }
}

fn router() -> axum::Router {
    Router::new(Invoker::new("test", ()))
        .route("/echo", get::<Echo, ()>())
        .route("/echo", post::<Echo, ()>())
        .route("/echo/{name}", get::<Echo, ()>())
        .route("/echo/{name}", post::<Echo, ()>())
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
    let invoker = Invoker::new("tenant", ());
    let invocation = Invocation::new(EchoInput {
        name: "core".to_string(),
        count: None,
    })
    .metadata(Metadata {
        correlation_id: Some("call-1".to_string()),
        ..Metadata::default()
    });

    let output = invoker.invoke::<Echo>(invocation).await.expect("operation succeeds");

    assert_eq!(output.owner, "tenant");
    assert_eq!(output.correlation_id.as_deref(), Some("call-1"));
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

#[derive(Clone, Copy)]
struct Accepted;

impl<P: Provider> Projector<Echo, P> for Accepted {
    fn output(&self, _output: EchoOutput) -> Response {
        StatusCode::ACCEPTED.into_response()
    }

    fn error(&self, _error: omnia_guest::Error) -> Response {
        StatusCode::IM_A_TEAPOT.into_response()
    }
}

#[tokio::test]
async fn projector() {
    let router = Router::new(Invoker::new("test", ()))
        .route("/echo", get_with::<Echo, (), Accepted>(Accepted))
        .into_axum();
    let request =
        Request::builder().uri("/echo?name=custom").body(Body::empty()).expect("build request");
    let response = router.oneshot(request).await.expect("router serves request");

    assert_eq!(response.status(), StatusCode::ACCEPTED);
}

#[tokio::test]
async fn route_state_clones_share_provider() {
    let router = Router::new(Invoker::new(
        "test",
        StatefulProvider {
            calls: AtomicUsize::new(0),
        },
    ))
    .route("/first", get::<ObserveProvider, StatefulProvider>())
    .route("/second", get::<ObserveProvider, StatefulProvider>())
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

#[test]
fn inventory() {
    let router = Router::new(Invoker::new("test", ()))
        .route("/echo", get::<Echo, ()>())
        .route("/echo", post::<Echo, ()>());
    let inventory = router.inventory();

    assert_eq!(inventory.len(), 2);
    assert_eq!(inventory[0].method(), Method::GET);
    assert_eq!(inventory[0].path(), "/echo");
    assert_eq!(inventory[0].operation(), TypeId::of::<Echo>());
    assert_eq!(inventory[1].method(), Method::POST);
}

#[derive(Clone, Copy)]
struct Capture;

impl DeliveryProjector<EchoOutput, omnia_guest::Error, serde_json::Error> for Capture {
    fn project(
        &self, outcome: DeliveryOutcome<EchoOutput, omnia_guest::Error, serde_json::Error>,
    ) -> Result<(), DeliveryError> {
        match outcome {
            DeliveryOutcome::Output(output)
                if output.name == "message"
                    && output.correlation_id.as_deref() == Some("delivery-1") =>
            {
                Ok(())
            }
            DeliveryOutcome::Output(_) => {
                Err(DeliveryError::Rejected("unexpected output".to_string()))
            }
            DeliveryOutcome::Operation(error) => {
                Err(DeliveryError::Rejected(format!("operation: {error}")))
            }
            DeliveryOutcome::Decode(error) => {
                Err(DeliveryError::Rejected(format!("decode: {error}")))
            }
        }
    }
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
    let router = MessagingRouter::new(Invoker::new("messages", ()))
        .route("events.created", consume::<Echo>().project_with(Capture));

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
        MessagingRouter::new(Invoker::new("messages", ())).route("events", consume::<Echo>());

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
fn messaging_inventory() {
    let router = MessagingRouter::new(Invoker::new("messages", ()))
        .route("events.created", consume::<Echo>());

    assert_eq!(router.inventory()[0].topic(), "events.created");
    assert_eq!(router.inventory()[0].operation(), TypeId::of::<Echo>());
}

#[test]
#[should_panic(expected = "duplicate messaging topic")]
fn messaging_duplicate_topic() {
    let _router = MessagingRouter::new(Invoker::new("messages", ()))
        .route("events", consume::<Echo>())
        .route("events", consume::<Echo>());
}
