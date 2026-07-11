//! # Conformance Wasm Guest
//!
//! Purpose-built guest for the `omnia-seam-suite` integration tests: one
//! component that exposes a route per HTTP-driven WASI interface and imports
//! the real guest APIs, so a single runtime fixture can drive every seam.
//!
//! Route identifiers (keys, object names, document ids) come from the request,
//! letting concurrent tests share one runtime without colliding.

#![cfg(target_arch = "wasm32")]

use anyhow::{Context, anyhow};
use axum::extract::{Path, Query};
use axum::routing::{get, post};
use axum::{Json, Router};
use bytes::Bytes;
use omnia_guest::document_store::{
    Document as DocDocument, Filter as DocFilter, QueryOptions as DocQueryOptions, SortField,
};
use omnia_guest::{DocumentStore, HttpResult};
use omnia_wasi_blobstore::blobstore;
use omnia_wasi_blobstore::types::{IncomingValue, OutgoingValue};
use omnia_wasi_config::store as config_store;
use omnia_wasi_identity::credentials::get_identity;
use omnia_wasi_keyvalue::atomics::{self, Cas, CasError};
use omnia_wasi_keyvalue::store as kv_store;
use omnia_wasi_messaging::producer;
use omnia_wasi_messaging::types::{Client as MessagingClient, Message};
use omnia_wasi_sql::readwrite;
use omnia_wasi_sql::types::{Connection, DataType, Statement};
use omnia_wasi_vault::vault;
use omnia_wasi_websocket::client as ws_client;
use omnia_wasi_websocket::types::{Client as WsClient, Error as WsHandlerError, Event};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tracing::Level;
use wasip3::exports::http::handler::Guest;
use wasip3::http::types::{ErrorCode, Request, Response};

struct Http;
wasip3::http::service::export!(Http);

impl Guest for Http {
    #[omnia_wasi_otel::instrument(name = "http_guest_handle", level = Level::DEBUG)]
    async fn handle(request: Request) -> Result<Response, ErrorCode> {
        let router = Router::new()
            .route("/echo", post(echo))
            .route("/keyvalue", post(keyvalue_round_trip))
            .route("/blobstore", post(blobstore_round_trip))
            .route("/config", get(config_get_all))
            .route("/identity", get(identity_token))
            .route("/sql/agencies", post(sql_insert_agency))
            .route("/vault", post(vault_round_trip))
            .route("/messaging/pub-sub", post(messaging_publish))
            .route("/websocket", post(websocket_send))
            .route("/otel", post(otel_emit))
            .route("/docstore/stops", get(docstore_list_stops).post(docstore_create_stop))
            .route("/docstore/stops/{id}", get(docstore_get_stop).delete(docstore_delete_stop));
        omnia_wasi_http::serve(router, request).await
    }
}

// --- wasi:http ---

#[omnia_wasi_otel::instrument]
async fn echo(Json(body): Json<Value>) -> HttpResult<Json<Value>> {
    Ok(Json(json!({ "message": "echo", "request": body })))
}

// --- wasi:keyvalue (store + atomics CAS legs) ---

#[derive(Debug, Deserialize)]
struct KeyValueParams {
    key: String,
    cas: String,
}

#[omnia_wasi_otel::instrument]
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

// --- wasi:blobstore (streaming write, then read back) ---

#[derive(Debug, Deserialize)]
struct BlobstoreParams {
    object: String,
}

#[omnia_wasi_otel::instrument]
async fn blobstore_round_trip(
    Query(p): Query<BlobstoreParams>, body: Bytes,
) -> HttpResult<Json<Value>> {
    let outgoing = OutgoingValue::new_outgoing_value();
    {
        let stream = outgoing
            .outgoing_value_write_body()
            .await
            .map_err(|()| anyhow!("failed to create stream"))?;
        stream.blocking_write_and_flush(&body).map_err(|e| anyhow!("writing body: {e}"))?;
    }

    let container = blobstore::create_container("container".to_string())
        .await
        .map_err(|e| anyhow!("failed to create container: {e}"))?;
    container
        .write_data(p.object.clone(), &outgoing)
        .await
        .map_err(|e| anyhow!("failed to write data: {e}"))?;
    OutgoingValue::finish(outgoing).map_err(|e| anyhow!("issue finishing: {e}"))?;

    let incoming = container
        .get_data(p.object.clone(), 0, 0)
        .await
        .map_err(|e| anyhow!("failed to read data: {e}"))?;
    let data = IncomingValue::incoming_value_consume_sync(incoming)
        .map_err(|e| anyhow!("failed to consume incoming value: {e}"))?;
    if data != body {
        Err(anyhow!("blob round-trip mismatch"))?;
    }

    let response =
        serde_json::from_slice::<Value>(&data).map_err(|e| anyhow!("deserializing data: {e}"))?;
    Ok(Json(response))
}

// --- wasi:config ---

#[omnia_wasi_otel::instrument]
async fn config_get_all() -> HttpResult<Json<Value>> {
    let config = config_store::get_all().map_err(|e| anyhow!("getting config: {e:?}"))?;
    Ok(Json(json!({ "config": config })))
}

// --- wasi:identity ---

#[omnia_wasi_otel::instrument]
async fn identity_token() -> HttpResult<Json<Value>> {
    let identity = get_identity("identity".to_string()).await.context("getting identity")?;
    let scopes = vec!["https://management.azure.com/.default".to_string()];
    let token = identity.get_token(scopes).await.context("getting access token")?;
    if token.token.is_empty() {
        Err(anyhow!("token is empty"))?;
    }
    Ok(Json(json!({ "message": "token acquired" })))
}

// --- wasi:sql (prepare/exec/query with parameters) ---

#[derive(Debug, Deserialize)]
struct CreateAgencyRequest {
    name: String,
}

#[omnia_wasi_otel::instrument]
async fn sql_insert_agency(Json(req): Json<CreateAgencyRequest>) -> HttpResult<Json<Value>> {
    let pool = Connection::open("db".to_string())
        .await
        .map_err(|e| anyhow!("failed to open connection: {}", e.trace()))?;

    let create = "CREATE TABLE IF NOT EXISTS agency (agency_id INTEGER PRIMARY KEY, name TEXT \
                  NOT NULL)";
    let stmt = Statement::prepare(create.to_string(), vec![])
        .await
        .map_err(|e| anyhow!("preparing create table: {}", e.trace()))?;
    readwrite::exec(&pool, &stmt).await.map_err(|e| anyhow!("creating table: {}", e.trace()))?;

    let insert = "INSERT INTO agency (name) VALUES ($1)";
    let stmt = Statement::prepare(insert.to_string(), vec![DataType::Str(Some(req.name.clone()))])
        .await
        .map_err(|e| anyhow!("preparing insert: {}", e.trace()))?;
    readwrite::exec(&pool, &stmt).await.map_err(|e| anyhow!("inserting agency: {}", e.trace()))?;

    let select = "SELECT name FROM agency WHERE name = $1";
    let stmt = Statement::prepare(select.to_string(), vec![DataType::Str(Some(req.name.clone()))])
        .await
        .map_err(|e| anyhow!("preparing select: {}", e.trace()))?;
    let rows =
        readwrite::query(&pool, &stmt).await.map_err(|e| anyhow!("querying: {}", e.trace()))?;
    if rows.len() != 1 {
        Err(anyhow!("inserted agency is not selectable"))?;
    }

    Ok(Json(json!({ "agency": { "name": req.name } })))
}

// --- wasi:vault ---

#[derive(Debug, Deserialize)]
struct VaultParams {
    secret: String,
}

#[omnia_wasi_otel::instrument]
async fn vault_round_trip(Query(p): Query<VaultParams>, body: Bytes) -> HttpResult<Json<Value>> {
    let locker = vault::open("omnia-locker".to_string()).await.context("opening locker")?;
    locker.set(p.secret.clone(), body.to_vec()).await.context("setting secret")?;
    let secret = locker.get(p.secret.clone()).await.context("reading secret")?;
    if secret.as_deref() != Some(body.as_ref()) {
        Err(anyhow!("vault round-trip mismatch"))?;
    }

    let response = serde_json::from_slice::<Value>(&body).context("deserializing data")?;
    Ok(Json(response))
}

// --- wasi:messaging (publish to topic `a`) ---

#[omnia_wasi_otel::instrument]
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

#[omnia_wasi_otel::instrument]
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

// --- wasi:otel (metrics via both the tracing and native OTel APIs) ---

#[omnia_wasi_otel::instrument]
async fn otel_emit(Json(body): Json<Value>) -> HttpResult<Json<Value>> {
    tracing::info!(monotonic_counter.conformance_counter = 1, "seam metric");
    tracing::info!(gauge.conformance_gauge = 1);

    let meter = opentelemetry::global::meter("conformance");
    let counter = meter.u64_counter("conformance_otel_counter").build();
    counter.add(1, &[opentelemetry::KeyValue::new("key1", "value 1")]);

    Ok(Json(json!({ "message": "telemetry emitted", "request": body })))
}

// --- wasi:docstore (insert/get/query/delete with a filter) ---

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Stop {
    stop_name: String,
    zone_id: Option<String>,
}

#[derive(Debug, Deserialize)]
struct CreateStopRequest {
    id: String,
    #[serde(flatten)]
    stop: Stop,
}

#[derive(Debug, Deserialize)]
struct StopQuery {
    zone: Option<String>,
    limit: Option<u32>,
    continuation: Option<String>,
}

#[omnia_wasi_otel::instrument]
async fn docstore_create_stop(Json(req): Json<CreateStopRequest>) -> HttpResult<Json<Value>> {
    let doc = DocDocument {
        id: req.id.clone(),
        data: serde_json::to_vec(&req.stop).context("serializing stop")?,
    };
    DocProvider.insert("stops", &doc).await.context("inserting stop")?;
    Ok(Json(json!({ "stop": req.stop, "id": req.id })))
}

#[omnia_wasi_otel::instrument]
async fn docstore_get_stop(Path(id): Path<String>) -> HttpResult<Json<Value>> {
    let doc = DocProvider
        .get("stops", &id)
        .await
        .context("fetching stop")?
        .ok_or_else(|| anyhow!("stop not found"))?;
    let stop: Stop = serde_json::from_slice(&doc.data).context("deserializing stop")?;
    Ok(Json(json!({ "id": doc.id, "stop": stop })))
}

#[omnia_wasi_otel::instrument]
async fn docstore_delete_stop(Path(id): Path<String>) -> HttpResult<Json<Value>> {
    let removed = DocProvider.delete("stops", &id).await.context("deleting stop")?;
    if !removed {
        return Err(anyhow!("stop not found").into());
    }
    Ok(Json(json!({ "message": "stop deleted", "id": id })))
}

#[omnia_wasi_otel::instrument]
async fn docstore_list_stops(Query(p): Query<StopQuery>) -> HttpResult<Json<Value>> {
    let filter = p.zone.as_ref().map(|zone| DocFilter::eq("zone_id", zone.as_str()));

    let result = DocProvider
        .query(
            "stops",
            DocQueryOptions {
                filter,
                order_by: vec![SortField {
                    field: "stop_name".into(),
                    descending: false,
                }],
                limit: p.limit,
                continuation: p.continuation,
                ..Default::default()
            },
        )
        .await
        .context("querying stops")?;

    let stops = result
        .documents
        .iter()
        .map(|doc| {
            let mut val: Value =
                serde_json::from_slice(&doc.data).context("deserializing document")?;
            if let Value::Object(ref mut m) = val {
                m.insert("id".to_string(), Value::String(doc.id.clone()));
            }
            anyhow::Ok(val)
        })
        .collect::<anyhow::Result<Vec<_>>>()?;
    Ok(Json(json!({ "stops": stops, "continuation": result.continuation })))
}

struct DocProvider;

impl DocumentStore for DocProvider {}
