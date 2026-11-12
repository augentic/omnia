//! Exact-topic messaging routing contracts.

mod support;

use std::future::{Ready, ready};
use std::sync::{Arc, Mutex};

use omnia_sdk::api::messaging::{Delivery, DeliveryError};
use omnia_sdk::api::{Client, Context, DecodeError, Handlers};
use serde::{Deserialize, Serialize};

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
    correlation_id: Option<String>,
}

#[derive(Debug)]
struct RawText {
    text: String,
}

fn echo<P: Send + Sync + 'static>(
    context: Context<P>, input: EchoInput,
) -> Ready<Result<EchoOutput, omnia_sdk::Error>> {
    ready(Ok(EchoOutput {
        name: input.name,
        count: input.count.unwrap_or(1),
        correlation_id: context.metadata.correlation_id,
    }))
}

fn delivery(topic: Option<&str>, payload: &[u8]) -> Delivery {
    let mut delivery = Delivery::new(topic.unwrap_or_default(), payload)
        .with_content_type("application/json")
        .with_metadata("correlation-id", "delivery-1")
        .with_metadata("tenant", "acme");
    delivery.topic = topic.map(str::to_owned);
    delivery
}

#[tokio::test]
async fn exact_topic() {
    let router = Handlers::new().topic("events.created", echo).with_state(Client::new(()));

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
async fn malformed_delivery() {
    let router = Handlers::new().topic("events", echo).with_state(Client::new(()));

    assert_eq!(
        router.handle(delivery(None, br#"{"name":"message"}"#)).await,
        Err(DeliveryError::MissingTopic)
    );
    assert!(matches!(
        router.handle(delivery(Some("events"), b"not-json")).await,
        Err(DeliveryError::Rejected(_))
    ));
}

#[tokio::test]
async fn topic_span() {
    let (spans, _guard) = Spans::install();
    let router = Handlers::new().topic("events.created", echo).with_state(Client::new(()));
    let delivery = Delivery::new("events.created", br#"{"name":"message"}"#)
        .with_metadata("request-id", "req-1")
        .with_metadata("correlation-id", "corr-1");

    router.handle(delivery).await.expect("the route handles the delivery");

    assert_eq!(
        spans.handler(),
        fields([
            ("request_id", "req-1"),
            ("correlation_id", "corr-1"),
            ("topic", "events.created"),
        ])
    );
}

#[tokio::test]
async fn invalid_payload_span() {
    let (spans, _guard) = Spans::install();
    let router = Handlers::new().topic("events.created", echo).with_state(Client::new(()));
    let delivery = Delivery::new("events.created", b"not-json")
        .with_metadata("request-id", "req-1")
        .with_metadata("correlation-id", "corr-1");

    assert!(matches!(router.handle(delivery).await, Err(DeliveryError::Rejected(_))));
    assert_eq!(
        spans.handler(),
        fields([
            ("request_id", "req-1"),
            ("correlation_id", "corr-1"),
            ("topic", "events.created"),
        ])
    );
}

#[test]
#[should_panic(expected = "duplicate messaging topic")]
fn duplicate_topic() {
    let _handlers = Handlers::<()>::new().topic("events", echo).topic("events", echo);
}

#[tokio::test]
async fn merged_routes() {
    let created = Handlers::new().topic("events.created", echo);
    let deleted = Handlers::new().topic("events.deleted", echo);
    let updated = Handlers::new().topic("events.updated", echo);
    let router = Handlers::from([created, deleted, updated]).with_state(Client::new(()));

    for topic in ["events.created", "events.deleted", "events.updated"] {
        router
            .handle(delivery(Some(topic), br#"{"name":"message"}"#))
            .await
            .unwrap_or_else(|error| panic!("the merged route handles `{topic}`: {error}"));
    }
    assert_eq!(
        router.topics().collect::<Vec<_>>(),
        ["events.created", "events.deleted", "events.updated"]
    );
    assert_eq!(
        router.handle(delivery(Some("events"), br#"{"name":"message"}"#)).await,
        Err(DeliveryError::UnhandledTopic("events".to_string()))
    );
}

#[test]
#[should_panic(expected = "duplicate messaging topic `events`")]
fn duplicate_topic_merged() {
    let _handlers =
        Handlers::<()>::new().topic("events", echo).merge(Handlers::new().topic("events", echo));
}

fn raw_text<P: Send + Sync + 'static>(
    _context: Context<P>, input: RawText,
) -> Ready<Result<String, omnia_sdk::Error>> {
    ready(Ok(input.text))
}

#[tokio::test]
async fn topic_with_raw_payload() {
    let router = Handlers::new()
        .topic_with("events.raw", raw_text, |delivery: &Delivery| {
            std::str::from_utf8(&delivery.payload)
                .map(|text| RawText {
                    text: text.to_owned(),
                })
                .map_err(|error| DecodeError::new(format!("payload is not utf-8: {error}")))
        })
        .with_state(Client::new(()));

    router
        .handle(delivery(Some("events.raw"), b"not-json"))
        .await
        .expect("the raw decoder handles a payload the JSON route rejects");
}

#[tokio::test]
async fn tenant_metadata() {
    let seen = Arc::new(Mutex::new(None));
    let record = {
        let seen = Arc::clone(&seen);
        move |context: Context<()>, _input: EchoInput| {
            *seen.lock().expect("record") = context.metadata.tenant;
            ready(Ok::<(), omnia_sdk::Error>(()))
        }
    };
    let router = Handlers::new().topic("events", record).with_state(Client::new(()));

    router
        .handle(delivery(Some("events"), br#"{"name":"message"}"#))
        .await
        .expect("the route handles the delivery");

    assert_eq!(seen.lock().expect("seen").as_deref(), Some("acme"));
}
