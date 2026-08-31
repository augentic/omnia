//! # DocStore Wasm Guest (Default Backend)
//!
//! Demonstrates the `wasi:docstore` document store interface: CRUD on a
//! single `stops` collection plus one query endpoint showing filters
//! (`contains`, `eq`, `gte`/`lte`), sorting, limits, and continuation-token
//! pagination.

#![cfg(target_arch = "wasm32")]

use anyhow::{Context, Result, anyhow};
use axum::extract::{Path, Query};
use axum::routing::get;
use axum::{Json, Router};
use omnia_guest::document_store::{Document, Filter, QueryOptions, SortField};
use omnia_guest::{DocumentStore, HttpResult};
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
            .route("/stops", get(list_stops).post(create_stop))
            .route("/stops/{id}", get(get_stop).put(upsert_stop).delete(delete_stop));
        omnia_wasi_http::serve(router, request).await
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Stop {
    stop_name: String,
    stop_lat: f64,
    stop_lon: f64,
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
    q: Option<String>,
    zone: Option<String>,
    min_lat: Option<f64>,
    max_lat: Option<f64>,
    limit: Option<u32>,
    continuation: Option<String>,
}

#[axum::debug_handler]
#[omnia_wasi_otel::instrument]
async fn create_stop(Json(req): Json<CreateStopRequest>) -> HttpResult<Json<Value>> {
    let doc = Document {
        id: req.id.clone(),
        data: serde_json::to_vec(&req.stop).context("serializing stop")?,
    };
    Provider.insert("stops", &doc).await.context("inserting stop")?;
    Ok(Json(json!({ "stop": req.stop, "id": req.id })))
}

#[axum::debug_handler]
#[omnia_wasi_otel::instrument]
async fn get_stop(Path(id): Path<String>) -> HttpResult<Json<Value>> {
    let doc = Provider
        .get("stops", &id)
        .await
        .context("fetching stop")?
        .ok_or_else(|| anyhow!("stop not found"))?;
    let stop: Stop = serde_json::from_slice(&doc.data).context("deserializing stop")?;
    Ok(Json(json!({ "id": doc.id, "stop": stop })))
}

#[axum::debug_handler]
#[omnia_wasi_otel::instrument]
async fn upsert_stop(Path(id): Path<String>, Json(stop): Json<Stop>) -> HttpResult<Json<Value>> {
    let doc = Document {
        id: id.clone(),
        data: serde_json::to_vec(&stop).context("serializing stop")?,
    };
    Provider.put("stops", &doc).await.context("upserting stop")?;
    Ok(Json(json!({ "id": id, "stop": stop })))
}

#[axum::debug_handler]
#[omnia_wasi_otel::instrument]
async fn delete_stop(Path(id): Path<String>) -> HttpResult<Json<Value>> {
    let removed = Provider.delete("stops", &id).await.context("deleting stop")?;
    if !removed {
        return Err(anyhow!("stop not found").into());
    }
    Ok(Json(json!({ "message": "stop deleted", "id": id })))
}

#[axum::debug_handler]
#[omnia_wasi_otel::instrument]
async fn list_stops(Query(p): Query<StopQuery>) -> HttpResult<Json<Value>> {
    let mut filters = Vec::new();

    if let Some(q) = &p.q {
        filters.push(Filter::contains("stop_name", q));
    }
    if let Some(zone) = &p.zone {
        filters.push(Filter::eq("zone_id", zone.as_str()));
    }
    if let Some(v) = p.min_lat {
        filters.push(Filter::gte("stop_lat", v));
    }
    if let Some(v) = p.max_lat {
        filters.push(Filter::lte("stop_lat", v));
    }

    let filter = if filters.is_empty() { None } else { Some(Filter::and(filters)) };

    let result = Provider
        .query(
            "stops",
            QueryOptions {
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
            Ok(val)
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(Json(json!({ "stops": stops, "continuation": result.continuation })))
}

struct Provider;

impl DocumentStore for Provider {}
