//! Exact-topic messaging routing contracts.

use std::future::{Ready, ready};
use std::sync::{Arc, Mutex};

use omnia_sdk::api::messaging::{Delivery, DeliveryError};
use omnia_sdk::api::{Client, Context, DecodeError, Handlers, Router};
use serde::{Deserialize, Serialize};

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
    input: EchoInput, context: Context<P>,
) -> Ready<Result<EchoOutput, omnia_sdk::Error>> {
    ready(Ok(EchoOutput {
        name: input.name,
        count: input.count.unwrap_or(1),
        correlation_id: context.metadata.correlation_id,
    }))
}

fn delivery(topic: Option<&str>, payload: &[u8]) -> Delivery {
    Delivery {
        topic: topic.map(str::to_owned),
        payload: payload.to_vec(),
        content_type: Some("application/json".to_string()),
        metadata: vec![
            ("correlation-id".to_string(), "delivery-1".to_string()),
            ("tenant".to_string(), "acme".to_string()),
        ],
    }
}

#[tokio::test]
async fn messaging_exact_topic() {
    let router = Router::new(Client::new(())).topic("events.created", echo);

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
    let router = Router::new(Client::new(())).topic("events", echo);

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
fn dup_topic() {
    let _router = Router::new(Client::new(())).topic("events", echo).topic("events", echo);
}

#[tokio::test]
async fn messaging_merged_routes() {
    let created = Handlers::new().topic("events.created", echo);
    let deleted = Handlers::new().topic("events.deleted", echo);
    let router = Router::new(Client::new(()))
        .merge(Handlers::from([created, deleted]))
        .topic("events.updated", echo);

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
fn dup_topic_across() {
    let _handlers =
        Handlers::<()>::new().topic("events", echo).merge(Handlers::new().topic("events", echo));
}

#[test]
#[should_panic(expected = "duplicate messaging topic `events`")]
fn dup_topic_merged() {
    let _router = Router::new(Client::new(()))
        .topic("events", echo)
        .merge(Handlers::new().topic("events", echo));
}

fn raw_text<P: Send + Sync + 'static>(
    input: RawText, _context: Context<P>,
) -> Ready<Result<String, omnia_sdk::Error>> {
    ready(Ok(input.text))
}

#[tokio::test]
async fn topic_with_raw_payload() {
    let router =
        Router::new(Client::new(())).topic_with("events.raw", raw_text, |delivery: &Delivery| {
            std::str::from_utf8(&delivery.payload)
                .map(|text| RawText {
                    text: text.to_owned(),
                })
                .map_err(|error| DecodeError::new(format!("payload is not utf-8: {error}")))
        });

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
        move |_input: EchoInput, context: Context<()>| {
            *seen.lock().expect("record") = context.metadata.tenant;
            ready(Ok::<(), omnia_sdk::Error>(()))
        }
    };
    let router = Router::new(Client::new(())).topic("events", record);

    router
        .handle(delivery(Some("events"), br#"{"name":"message"}"#))
        .await
        .expect("the route handles the delivery");

    assert_eq!(seen.lock().expect("seen").as_deref(), Some("acme"));
}
