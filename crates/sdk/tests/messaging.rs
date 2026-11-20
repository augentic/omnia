//! Exact-topic messaging routing contracts.

mod support;

use std::collections::BTreeMap;
use std::fmt;
use std::future::{Ready, ready};
use std::sync::{Arc, Mutex};

use omnia_sdk::messaging::{Delivery, Message};
use omnia_sdk::{Context, ErrorKind, Handlers, bad_gateway, bad_request, model, plugins, vcs};
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

fn echo(context: Context, input: EchoInput) -> Ready<Result<EchoOutput, omnia_sdk::Error>> {
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
    let router = Handlers::new().topic("events.created", echo).into_router();

    let reply = router
        .handle(delivery(Some("events.created"), br#"{"name":"message","count":2}"#))
        .await
        .expect("exact route handles delivery");
    assert!(reply.is_none(), "a topic route replied: {reply:?}");
    assert_eq!(
        router.handle(delivery(Some("events.*"), br#"{"name":"message"}"#)).await.expect_err("*"),
        omnia_sdk::Error::not_found("unhandled-topic", "no route for topic `events.*`")
    );
}

#[tokio::test]
async fn malformed_delivery() {
    let router = Handlers::new().topic("events", echo).into_router();

    assert_eq!(
        router.handle(delivery(None, br#"{"name":"message"}"#)).await.expect_err("no topic"),
        omnia_sdk::Error::bad_request("missing-topic", "the delivery has no topic")
    );
    let undecodable =
        router.handle(delivery(Some("events"), b"not-json")).await.expect_err("not JSON");
    assert_eq!((undecodable.kind(), undecodable.code()), (ErrorKind::BadRequest, "undecodable"));
}

#[derive(Debug, thiserror::Error)]
#[error("could not place the order")]
struct PlaceError(#[source] omnia_sdk::Error);

#[derive(Debug, thiserror::Error)]
#[error("could not review")]
struct ReviewError(#[source] model::Exhausted);

fn unavailable() -> omnia_sdk::Error {
    omnia_sdk::Error::bad_gateway("unavailable", "store did not answer")
}

#[tokio::test]
async fn failing_handler() {
    let router = Handlers::new()
        .topic("gateway", |_: EchoInput| {
            ready(Err::<(), _>(bad_gateway!("upstream did not answer")))
        })
        .topic("store", |_: EchoInput| ready(Err::<(), _>(unavailable())))
        .topic("order", |_: EchoInput| ready(Err::<(), _>(PlaceError(unavailable()))))
        .topic("request", |_: EchoInput| ready(Err::<(), _>(bad_request!("no name"))))
        .topic("format", |_: EchoInput| ready(Err::<(), _>(fmt::Error)))
        .topic("revision", |_: EchoInput| {
            ready(Err::<(), _>(vcs::types::Error::NotFound("no revision `v1`".into())))
        })
        .topic("plugin", |_: EchoInput| {
            let conflict = plugins::Conflict {
                found: "sha256:ab".into(),
                current: None,
            };
            ready(Err::<(), _>(conflict))
        })
        .topic("review", |_: EchoInput| {
            let exhausted = model::Exhausted {
                budget: model::Budget::Tokens,
                detail: "4096 tokens".into(),
                usage: None,
            };
            ready(Err::<(), _>(ReviewError(exhausted)))
        })
        .into_router();

    for (topic, status, code, text) in [
        ("gateway", 502, "bad_gateway", "upstream did not answer"),
        ("store", 502, "unavailable", "store did not answer"),
        ("order", 502, "unavailable", "could not place the order: store did not answer"),
        ("request", 400, "bad_request", "no name"),
        ("format", 500, "server_error", "when formatting"),
        ("revision", 500, "not-found", "no revision `v1`"),
        ("plugin", 409, "conflict", "not the pinned digest"),
        ("review", 400, "exhausted", "could not review: the tokens budget is exhausted"),
    ] {
        let Err(error) = router.handle(delivery(Some(topic), br#"{"name":"message"}"#)).await
        else {
            panic!("`{topic}` fails in its handler");
        };
        assert_eq!(error.status().as_u16(), status, "{topic}: {error}");
        assert_eq!(error.code(), code, "{topic}: {error}");
        assert!(error.description().contains(text), "{topic}: {error}");
    }
}

#[tokio::test]
async fn topic_span() {
    let (spans, _guard) = Spans::install();
    let router = Handlers::new().topic("events.created", echo).into_router();
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
    let router = Handlers::new().topic("events.created", echo).into_router();
    let delivery = Delivery::new("events.created", b"not-json")
        .with_metadata("request-id", "req-1")
        .with_metadata("correlation-id", "corr-1");

    let undecodable = router.handle(delivery).await.expect_err("not JSON");
    assert_eq!((undecodable.kind(), undecodable.code()), (ErrorKind::BadRequest, "undecodable"));
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
    let _handlers = Handlers::new().topic("events", echo).topic("events", echo);
}

#[tokio::test]
async fn merged_routes() {
    let created = Handlers::new().topic("events.created", echo);
    let deleted = Handlers::new().topic("events.deleted", echo);
    let updated = Handlers::new().topic("events.updated", echo);
    let router = Handlers::from([created, deleted, updated]).into_router();

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
        router
            .handle(delivery(Some("events"), br#"{"name":"message"}"#))
            .await
            .expect_err("events"),
        omnia_sdk::Error::not_found("unhandled-topic", "no route for topic `events`")
    );
}

#[test]
#[should_panic(expected = "duplicate messaging topic `events`")]
fn duplicate_topic_merged() {
    let _handlers =
        Handlers::new().topic("events", echo).merge(Handlers::new().topic("events", echo));
}

fn raw_text(input: RawText) -> Ready<Result<String, omnia_sdk::Error>> {
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
                .map_err(|error| bad_request!("payload is not utf-8: {error}"))
        })
        .into_router();

    router
        .handle(delivery(Some("events.raw"), b"not-json"))
        .await
        .expect("the raw decoder handles a payload the JSON route rejects");
    let refused =
        router.handle(delivery(Some("events.raw"), b"\xff")).await.expect_err("not utf-8");
    assert_eq!((refused.kind(), refused.code()), (ErrorKind::BadRequest, "undecodable"));
    assert!(refused.description().starts_with("payload is not utf-8"), "{refused}");
}

#[tokio::test]
async fn topic_with_reply_to() {
    let seen = Arc::new(Mutex::new(None));
    let record = {
        let seen = Arc::clone(&seen);
        move |input: (Option<String>, Option<u32>)| {
            *seen.lock().expect("record") = Some(input);
            ready(Ok::<(), omnia_sdk::Error>(()))
        }
    };
    let router = Handlers::new()
        .topic_with("quotes", record, |delivery: &Delivery| {
            Ok((delivery.reply_to.clone(), delivery.attempt))
        })
        .into_router();

    router
        .handle(delivery(Some("quotes"), b"{}").with_reply_to("quotes.reply").with_attempt(2))
        .await
        .expect("the route handles the delivery");

    let seen = seen.lock().expect("seen").clone();
    assert_eq!(seen, Some((Some("quotes.reply".to_owned()), Some(2))));
}

#[tokio::test]
async fn tenant_metadata() {
    let seen = Arc::new(Mutex::new(None));
    let record = {
        let seen = Arc::clone(&seen);
        move |context: Context, _input: EchoInput| {
            *seen.lock().expect("record") = context.metadata.tenant;
            ready(Ok::<(), omnia_sdk::Error>(()))
        }
    };
    let router = Handlers::new().topic("events", record).into_router();

    router
        .handle(delivery(Some("events"), br#"{"name":"message"}"#))
        .await
        .expect("the route handles the delivery");

    assert_eq!(seen.lock().expect("seen").as_deref(), Some("acme"));
}

#[derive(Debug, Deserialize)]
struct QuoteRequest {
    sku: String,
    quantity: u32,
}

#[derive(Debug, Serialize)]
struct Quote {
    sku: String,
    total_cents: u32,
}

fn quote(input: QuoteRequest) -> Ready<Result<Quote, omnia_sdk::Error>> {
    ready(Ok(Quote {
        sku: input.sku,
        total_cents: input.quantity * 125,
    }))
}

#[tokio::test]
async fn request_reply() {
    let router = Handlers::new().request("quotes", quote).into_router();

    let reply = router
        .handle(delivery(Some("quotes"), br#"{"sku":"apple","quantity":3}"#))
        .await
        .expect("the route handles the request")
        .expect("a request route replies");

    assert_eq!(reply.data, br#"{"sku":"apple","total_cents":375}"#);
    assert_eq!(reply.content_type.as_deref(), Some("application/json"));
}

#[tokio::test]
async fn request_with_message() {
    let echo = |text: String| {
        ready(Ok::<_, omnia_sdk::Error>(Message::new(text).with_content_type("text/plain")))
    };
    let router = Handlers::new()
        .request_with("echo", echo, |delivery: &Delivery| {
            Ok(String::from_utf8_lossy(&delivery.payload).into_owned())
        })
        .into_router();

    let reply = router
        .handle(delivery(Some("echo"), b"ping"))
        .await
        .expect("the route handles the request")
        .expect("a request route replies");

    assert_eq!(reply.data, b"ping");
    assert_eq!(reply.content_type.as_deref(), Some("text/plain"));
}

#[tokio::test]
async fn unencodable_reply() {
    let pairs = |_: EchoInput| ready(Ok::<_, omnia_sdk::Error>(BTreeMap::from([((1, 2), 3)])));
    let router = Handlers::new().request("pairs", pairs).into_router();

    let error = router
        .handle(delivery(Some("pairs"), br#"{"name":"message"}"#))
        .await
        .expect_err("a tuple key has no JSON form");

    assert_eq!((error.kind(), error.code()), (ErrorKind::ServerError, "invalid"));
}
