//! # Conformance Wasm Guest
//!
//! Purpose-built guest for the `omnia-seam-suite` integration tests: one
//! HTTP-triggered component exposing the routes the suite's conformance
//! scenarios drive — outbound proxying, the keyvalue CAS legs, a websocket
//! send, and a messaging publish — plus a websocket handler that records
//! inbound peer events where the host can observe them.

#![cfg(target_arch = "wasm32")]

use anyhow::{Context, anyhow};
use axum::extract::Query;
use axum::routing::post;
use axum::{Json, Router};
use bytes::Bytes;
use omnia_guest::HttpResult;
use omnia_wasi_keyvalue::atomics::{self, Cas, CasError};
use omnia_wasi_keyvalue::store as kv_store;
use omnia_wasi_messaging::producer;
use omnia_wasi_messaging::types::{Client as MessagingClient, Message};
use omnia_wasi_websocket::client as ws_client;
use omnia_wasi_websocket::types::{Client as WsClient, Error as WsHandlerError, Event};
use serde::Deserialize;
use serde_json::{Value, json};
use wasip3::exports::http::handler::Guest;
use wasip3::http::types::{ErrorCode, Request, Response};

struct Http;
wasip3::http::service::export!(Http);

impl Guest for Http {
    async fn handle(request: Request) -> Result<Response, ErrorCode> {
        let router = Router::new()
            .route("/proxy", post(proxy))
            .route("/keyvalue", post(keyvalue_round_trip))
            .route("/messaging/pub-sub", post(messaging_publish))
            .route("/websocket", post(websocket_send));
        omnia_wasi_http::serve(router, request).await
    }
}

// --- wasi:http (outbound) ---

#[derive(Debug, Deserialize)]
struct ProxyRequest {
    url: String,
    #[serde(default)]
    method: Option<String>,
    #[serde(default)]
    headers: Vec<(String, String)>,
    #[serde(default)]
    body: String,
}

/// Perform an outbound `wasi:http` request described by the JSON body and
/// report the outcome, so seam tests can observe the host's outbound hook
/// through the real guest boundary.
async fn proxy(Json(req): Json<ProxyRequest>) -> HttpResult<Json<Value>> {
    let method = req.method.as_deref().unwrap_or("GET");
    let mut builder = http::Request::builder().method(method).uri(&req.url);
    for (name, value) in &req.headers {
        builder = builder.header(name, value);
    }
    let outbound = match builder.body(http_body_util::Full::new(Bytes::from(req.body))) {
        Ok(outbound) => outbound,
        Err(e) => return Ok(Json(json!({ "error": format!("{e:#}") }))),
    };

    match omnia_wasi_http::handle(outbound).await {
        Ok(response) => {
            let headers: Vec<(String, String)> = response
                .headers()
                .iter()
                .map(|(k, v)| (k.to_string(), String::from_utf8_lossy(v.as_bytes()).into_owned()))
                .collect();
            Ok(Json(json!({
                "status": response.status().as_u16(),
                "headers": headers,
                "body": String::from_utf8_lossy(response.body()).into_owned(),
            })))
        }
        Err(e) => Ok(Json(json!({ "error": format!("{e:#}") }))),
    }
}

// --- wasi:keyvalue (store + atomics CAS legs) ---

#[derive(Debug, Deserialize)]
struct KeyValueParams {
    key: String,
    cas: String,
}

async fn keyvalue_round_trip(
    Query(p): Query<KeyValueParams>, body: Bytes,
) -> HttpResult<Json<Value>> {
    let bucket = kv_store::open("omnia_bucket".to_string()).await.context("opening bucket")?;

    bucket.set(p.key.clone(), body.to_vec()).await.context("storing data")?;
    let stored = bucket.get(p.key.clone()).await.context("reading data")?;
    if stored.as_deref() != Some(body.as_ref()) {
        Err(anyhow!("set/get round-trip mismatch"))?;
    }

    // CAS happy path: swap against an unchanged snapshot succeeds.
    bucket.set(p.cas.clone(), body.to_vec()).await.context("seeding cas key")?;
    let cas = Cas::new(&bucket, p.cas.clone()).await.context("creating cas")?;
    atomics::swap(cas, b"swapped".to_vec())
        .await
        .map_err(|e| anyhow!("swap on a fresh snapshot failed: {e:?}"))?;

    // CAS stale path: invalidate the snapshot, then retry with the fresh
    // handle the failure carries.
    let cas = Cas::new(&bucket, p.cas.clone()).await.context("creating stale cas")?;
    bucket.set(p.cas.clone(), b"interfering".to_vec()).await.context("interfering")?;
    match atomics::swap(cas, b"lost-race".to_vec()).await {
        Err(CasError::CasFailed(fresh)) => {
            atomics::swap(fresh, b"retried".to_vec())
                .await
                .map_err(|e| anyhow!("retry with the fresh handle failed: {e:?}"))?;
        }
        Ok(()) => Err(anyhow!("stale swap unexpectedly succeeded"))?,
        Err(other) => Err(anyhow!("stale swap failed unexpectedly: {other:?}"))?,
    }

    Ok(Json(json!({ "message": "keyvalue ok" })))
}

// --- wasi:messaging (publish to topic `a`) ---

async fn messaging_publish(Json(body): Json<Value>) -> HttpResult<Json<Value>> {
    let client = MessagingClient::connect("default".to_string())
        .await
        .map_err(|e| anyhow!("connect: {e}"))?;
    let message = Message::new(&Bytes::from(body.to_string()));
    message.set_content_type("application/json");

    producer::send(&client, "a".to_string(), message)
        .await
        .map_err(|e| anyhow!("publishing to topic 'a': {e}"))?;

    Ok(Json(json!({ "message": "message published" })))
}

// --- wasi:websocket (connect + send an event) ---

async fn websocket_send(message: String) -> HttpResult<Json<Value>> {
    let client =
        WsClient::connect("default".to_string()).await.map_err(|e| anyhow!("connecting: {e}"))?;
    let event = Event::new(&message.into_bytes());
    ws_client::send(&client, event, None).await.map_err(|e| anyhow!("sending event: {e}"))?;

    Ok(Json(json!({ "message": "event sent" })))
}

struct WebSocket;
omnia_wasi_websocket::export!(WebSocket);

impl omnia_wasi_websocket::handler::Guest for WebSocket {
    // Inbound peer messages land here; mirror them into the keyvalue store so
    // the seam test can observe delivery from the host side.
    async fn handle(event: Event) -> Result<(), WsHandlerError> {
        let bucket = kv_store::open("omnia_bucket".to_string())
            .await
            .map_err(|e| WsHandlerError::Other(format!("opening bucket: {e}")))?;
        bucket
            .set("ws-inbound".to_string(), event.data())
            .await
            .map_err(|e| WsHandlerError::Other(format!("recording event: {e}")))?;
        Ok(())
    }
}
