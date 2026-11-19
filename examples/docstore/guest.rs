//! # DocStore Wasm Guest (Default Backend)
//!
//! Demonstrates the `omnia:docstore` document store interface: CRUD on a
//! single `stops` collection plus one query endpoint showing filters
//! (`contains`, `eq`, `gte`/`lte`), sorting, limits, and cursor pagination.

#![cfg(target_arch = "wasm32")]

use anyhow::{Context, Result, anyhow};
use axum::extract::{Path, Query};
use axum::routing::get;
use axum::{Json, Router};
use omnia_sdk::bad_request;
use omnia_sdk::docstore::collection::Collection;
use omnia_sdk::docstore::{self, Filter, SortField, store};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use wasip3::exports::http::handler::Guest;
use wasip3::http::types::{ErrorCode, Request, Response};

struct Http;
wasip3::http::service::export!(Http);

impl Guest for Http {
    async fn handle(request: Request) -> Result<Response, ErrorCode> {
        let router = Router::new()
            .route("/stops", get(list_stops).post(create_stop))
            .route("/stops/{id}", get(get_stop).put(upsert_stop).delete(delete_stop));
        omnia_sdk::http::serve(router, request).await
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
#[omnia_sdk::otel::instrument]
async fn create_stop(Json(req): Json<CreateStopRequest>) -> omnia_sdk::Result<Json<Value>> {
    if stops().await?.insert(&req.id, &req.stop).await.context("inserting stop")?.is_err() {
        return Err(bad_request!("stop {} already exists", req.id));
    }
    Ok(Json(json!({ "stop": req.stop, "id": req.id })))
}

#[axum::debug_handler]
#[omnia_sdk::otel::instrument]
async fn get_stop(Path(id): Path<String>) -> omnia_sdk::Result<Json<Value>> {
    let doc = stops()
        .await?
        .get(id)
        .await
        .context("fetching stop")?
        .ok_or_else(|| anyhow!("stop not found"))?;
    let stop: Stop = doc.decode().context("deserializing stop")?;
    Ok(Json(json!({ "id": doc.id, "stop": stop })))
}

#[axum::debug_handler]
#[omnia_sdk::otel::instrument]
async fn upsert_stop(
    Path(id): Path<String>, Json(stop): Json<Stop>,
) -> omnia_sdk::Result<Json<Value>> {
    stops().await?.upsert(&id, &stop).await.context("upserting stop")?;
    Ok(Json(json!({ "id": id, "stop": stop })))
}

#[axum::debug_handler]
#[omnia_sdk::otel::instrument]
async fn delete_stop(Path(id): Path<String>) -> omnia_sdk::Result<Json<Value>> {
    let removed = stops().await?.delete(id.clone(), None).await.context("deleting stop")??;
    if !removed {
        return Err(anyhow!("stop not found").into());
    }
    Ok(Json(json!({ "message": "stop deleted", "id": id })))
}

#[axum::debug_handler]
#[omnia_sdk::otel::instrument]
async fn list_stops(Query(p): Query<StopQuery>) -> omnia_sdk::Result<Json<Value>> {
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

    let query = docstore::Query {
        filter,
        order_by: vec![SortField {
            field: "stop_name".into(),
            descending: false,
        }],
        limit: p.limit,
        after: p.continuation,
        ..Default::default()
    };
    let page = stops().await?.find(&query).await.context("querying stops")?;

    let stops = page
        .documents
        .iter()
        .map(|doc| {
            let mut val: Value = doc.decode().context("deserializing document")?;
            if let Value::Object(ref mut m) = val {
                m.insert("id".to_string(), Value::String(doc.id.clone()));
            }
            Ok(val)
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(Json(json!({ "stops": stops, "continuation": page.next })))
}

async fn stops() -> Result<Collection> {
    store::open("stops".to_owned()).await.context("opening stops")
}
