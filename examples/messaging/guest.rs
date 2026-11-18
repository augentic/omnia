//! # Messaging Wasm Guest (Default Backend)
//!
//! This module demonstrates the `omnia:messaging` interface with the default
//! (in-memory) backend. It shows the same patterns as the Kafka and NATS
//! examples but without requiring external services.
//!
//! ## Two Interfaces
//!
//! This guest implements two interfaces:
//! 1. **HTTP Handler**: Exposes REST endpoints to trigger messaging operations
//! 2. **Messaging Handler**: Processes incoming messages from subscribed topics
//!
//! ## Patterns Demonstrated
//!
//! - **Pub-Sub**: Publish messages to topics, receive via subscription
//! - **Request-Reply**: Send message and wait for response
//! - **Fan-out**: Receive one message, produce many

#![cfg(target_arch = "wasm32")]

use std::time::Instant;

use anyhow::anyhow;
use axum::routing::post;
use axum::{Json, Router};
use bytes::Bytes;
use http_body_util::Full;
use omnia_wasi_messaging::types::{Client, Delivery, Error, Message};
use omnia_wasi_messaging::{producer, request_reply};
use serde_json::{Value, json};
use wasip3::exports::http::handler::Guest;
use wasip3::http::types::{ErrorCode, Request, Response};

pub struct Http;
wasip3::http::service::export!(Http);

impl Guest for Http {
    async fn handle(request: Request) -> Result<Response, ErrorCode> {
        let router = Router::new()
            .route("/pub-sub", post(pub_sub))
            .route("/request-reply", post(request_reply_handler))
            .route("/upstream", post(upstream_handler));
        omnia_wasi_http::serve(router, request).await
    }
}

async fn pub_sub(Json(body): Json<Value>) -> omnia_sdk::Result<Json<Value>> {
    tracing::debug!("sending message to topic 'a'");

    let client =
        Client::connect("default".to_string()).await.map_err(|e| anyhow!("connect: {e}"))?;
    let outgoing = Message {
        content_type: Some("application/json".to_string()),
        key: Some("example_key".to_string()),
        ..message(body.to_string())
    };

    producer::send(&client, "a".to_string(), outgoing)
        .await
        .map_err(|e| anyhow!("publishing to topic 'a': {e}"))?;

    Ok(Json(json!({"message": "message published"})))
}

async fn request_reply_handler(body: Bytes) -> omnia_sdk::Result<Json<Value>> {
    let client =
        Client::connect("default".to_string()).await.map_err(|e| anyhow!("connect: {e}"))?;
    let reply = request_reply::request(&client, "c".to_string(), message(body), None)
        .await
        .map_err(|e| anyhow!("request-reply: {e}"))?;

    let data_str = String::from_utf8_lossy(&reply.data);

    Ok(Json(json!({"reply": data_str})))
}

// the target for outbound http from the messaging handler
async fn upstream_handler(body: Bytes) -> Json<Value> {
    Json(json!({"echo": String::from_utf8_lossy(&body)}))
}

fn message(data: impl Into<Vec<u8>>) -> Message {
    Message {
        data: data.into(),
        content_type: None,
        id: None,
        key: None,
        metadata: Vec::new(),
    }
}

pub struct Messaging;
omnia_wasi_messaging::export!(Messaging with_types_in omnia_wasi_messaging);

impl omnia_wasi_messaging::incoming_handler::Guest for Messaging {
    async fn handle(delivery: Delivery) -> Result<(), Error> {
        tracing::debug!("start processing msg");

        let topic = delivery.topic.clone();
        tracing::debug!("message received for: {topic}");

        match topic.as_str() {
            "a" => {
                tracing::debug!("handling topic a");

                let timer = Instant::now();

                for i in 0..1000 {
                    wit_bindgen::spawn_local(async move {
                        tracing::debug!("sending message iteration {i}");
                        let Ok(client) = Client::connect("default".to_string()).await else {
                            tracing::error!("failed to connect default client");
                            return;
                        };

                        let keyed = Message {
                            key: Some(format!("key-{i}")),
                            ..message(format!("topic a iteration {i}"))
                        };

                        if let Err(e) = producer::send(&client, "b".to_string(), keyed).await {
                            tracing::error!("error sending message to topic 'b': {e}");
                        }
                        tracing::debug!("message iteration {i} sent");

                        if i % 100 == 0 {
                            wit_bindgen::yield_async().await;
                            println!("sent 100 messages");
                        }
                    });
                }

                // the sends run in spawned tasks, so the timer covers spawning, not delivery
                println!("spawned 1000 sends in {} milliseconds", timer.elapsed().as_millis());
            }
            "b" => {
                tracing::debug!("handling topic b");
            }
            "c" => {
                let data_str = String::from_utf8(delivery.message.data)
                    .map_err(|e| Error::Other(format!("not utf8: {e}")))?;
                tracing::debug!("message received on topic 'c': {data_str}");

                // HACK!: outbound http here proves wasip3's wit-bindgen matches the
                // workspace's; a mismatch deadlocks, its body writer landing in a
                // different spawned queue than the active executor
                let upstream = http::Request::builder()
                    .method(http::Method::POST)
                    .uri("http://localhost:8080/upstream")
                    .body(Full::new(Bytes::from(data_str.clone())))
                    .map_err(|e| Error::Other(format!("building request: {e}")))?;

                println!("calling upstream HTTP from messaging handler");
                let upstream_body = match omnia_wasi_http::handle(upstream).await {
                    Ok(resp) => {
                        println!("upstream responded: {}", resp.status());
                        String::from_utf8_lossy(resp.body()).to_string()
                    }
                    Err(e) => {
                        println!("upstream call failed: {e}");
                        format!("error: {e}")
                    }
                };

                let combined =
                    format!("Hello from topic c | input: {data_str} | upstream: {upstream_body}");

                if let Some(reply_to) = delivery.reply_to {
                    request_reply::reply(reply_to, message(combined)).await?;
                }
            }
            _ => {
                tracing::debug!("unknown topic: {topic}");
            }
        }

        tracing::debug!("finished processing msg");
        Ok(())
    }
}
