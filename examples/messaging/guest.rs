//! # Messaging Wasm Guest (Default Backend)
//!
//! This module demonstrates the WASI Messaging interface with the default
//! (in-memory) backend. It shows the same patterns as the Kafka and NATS
//! examples but without requiring external services.
//!
//! ## Two Interfaces
//!
//! This guest implements two WASI interfaces:
//! 1. **HTTP Handler**: Exposes REST endpoints to trigger messaging operations
//! 2. **Messaging Handler**: Processes incoming messages from subscribed topics
//!
//! ## Patterns Demonstrated
//!
//! - **Pub-Sub**: Publish messages to topics, receive via subscription
//! - **Request-Reply**: Send message and wait for response
//! - **Fan-out**: Receive one message, produce many
//!
//! ## Note
//!
//! This example uses the default backend for pub-sub but references "nats"
//! for request-reply. In production, use consistent client names.

#![cfg(target_arch = "wasm32")]

use std::time::Instant;

use axum::routing::post;
use axum::{Json, Router};
use bytes::Bytes;
use qwasr_sdk::HttpResult;
use qwasr_wasi_messaging::types::{Client, Error, Message};
use qwasr_wasi_messaging::{producer, request_reply};
use serde_json::{Value, json};
use wasip3::exports::http::handler::Guest;
use wasip3::http::types::{ErrorCode, Request, Response};

// ----------------------------------------------------------------------------
// HTTP Interface
// ----------------------------------------------------------------------------

pub struct Http;
wasip3::http::service::export!(Http);

impl Guest for Http {
    /// Routes HTTP requests to messaging operations.
    async fn handle(request: Request) -> Result<Response, ErrorCode> {
        let router = Router::new()
            .route("/pub-sub", post(pub_sub))
            .route("/request-reply", post(request_reply_handler));
        qwasr_wasi_http::serve(router, request).await
    }
}

/// Publishes a message using the pub-sub pattern.
async fn pub_sub(Json(body): Json<Value>) -> HttpResult<Json<Value>> {
    tracing::debug!("sending message to topic 'a'");

    let client = Client::connect("default".to_string()).await.expect("should connect");
    let message = Message::new(&Bytes::from(body.to_string()));
    message.set_content_type("application/json");
    message.add_metadata("key", "example_key");

    wit_bindgen::block_on(async move {
        if let Err(e) = producer::send(&client, "a".to_string(), message).await {
            tracing::error!("error sending message to topic 'a': {e}");
        }
        tracing::debug!("handler: message published to topic 'a'");
    });

    Ok(Json(json!({"message": "message published"})))
}

/// Sends a message and waits for a reply.
async fn request_reply_handler(body: Bytes) -> Json<Value> {
    let client = Client::connect("default".to_string()).await.expect("should connect");
    let message = Message::new(&body);
    let reply = wit_bindgen::block_on(async move {
        request_reply::request(&client, "a".to_string(), &message, None).await
    })
    .expect("should reply");

    let data = reply[0].data();
    let data_str = String::from_utf8_lossy(&data);

    Json(json!({"reply": data_str}))
}

// ----------------------------------------------------------------------------
// Messaging Interface
// ----------------------------------------------------------------------------

pub struct Messaging;
qwasr_wasi_messaging::export!(Messaging with_types_in qwasr_wasi_messaging);

impl qwasr_wasi_messaging::incoming_handler::Guest for Messaging {
    /// Handles incoming messages from subscribed topics.
    async fn handle(message: Message) -> anyhow::Result<(), Error> {
        tracing::debug!("start processing msg");

        let topic = message.topic().unwrap_or_default();
        tracing::debug!("message received for: {topic}");

        match topic.as_str() {
            "a" => {
                tracing::debug!("handling topic a");

                let mut resp = b"topic a says: ".to_vec();
                resp.extend(message.data());

                let pubmsg = Message::new(&resp);
                if let Some(md) = message.metadata() {
                    pubmsg.set_metadata(&md);
                }
                if let Some(format) = message.content_type() {
                    pubmsg.set_content_type(&format);
                }

                let timer = Instant::now();

                for i in 0..1000 {
                    wit_bindgen::spawn(async move {
                        tracing::debug!("sending message iteration {i}");
                        let Ok(client) = Client::connect("default".to_string()).await else {
                            tracing::error!("failed to connect default client");
                            return;
                        };

                        let data = format!("topic a iteration {i}");
                        let message = Message::new(data.as_bytes());
                        message.add_metadata("key", &format!("key-{i}"));

                        if let Err(e) = producer::send(&client, "b".to_string(), message).await {
                            tracing::error!("error sending message to topic 'b': {e}");
                        }
                        tracing::debug!("message iteration {i} sent");

                        if i % 100 == 0 {
                            wit_bindgen::yield_async().await;
                            println!("sent 100 messages");
                        }
                    });
                }

                println!("sent 1000 messages in {} milliseconds", timer.elapsed().as_millis());
            }
            "b" => {
                tracing::debug!("handling topic b");
            }
            "c" => {
                let data = message.data();
                let data_str = String::from_utf8(data.clone())
                    .map_err(|e| Error::Other(format!("not utf8: {e}")))?;
                tracing::debug!("message received on topic 'c': {data_str}");

                let mut resp = b"Hello from topic c: ".to_vec();
                resp.extend(data);

                let reply = Message::new(&resp);
                request_reply::reply(&message, reply).await?;
            }
            _ => {
                tracing::debug!("unknown topic: {topic}");
            }
        }

        tracing::debug!("finished processing msg");
        Ok(())
    }
}
