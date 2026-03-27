//! # JsonDb Wasm Guest (Default Backend)
//!
//! Demonstrates the `wasi:jsondb` document store interface with GTFS-like data.
//! Three collections -- stops, routes, stop_times -- exercise all CRUD operations
//! and most filter types through combined query endpoints.
//!
//! ## Filter coverage
//!
//! - `eq`, `gte`, `lte`, `contains` -- stops query params
//! - `ne` -- stops `exclude_zone` param (direct `ComparisonOp::Ne` codepath)
//! - `in_list` -- routes `types` param
//! - `is_not_null`, `is_null` -- stops `accessible` and `top_level` params
//! - `or`, `contains` -- nested inside routes `q` param
//! - `not` -- nested inside routes `exclude_type` param
//! - `not(and(...))` -- routes `not_agency` + `not_type` combo (De Morgan negation)
//! - `on_date` -- stops `updated_on` param

#![cfg(target_arch = "wasm32")]

use anyhow::{Context, Result, anyhow};
use axum::extract::{Path, Query};
use axum::routing::get;
use axum::{Json, Router};
use omnia_sdk::document_store::{Document, Filter, QueryOptions, ScalarValue, SortField};
use omnia_sdk::{DocumentStore, HttpResult};
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
            .route("/stops/{id}", get(get_stop).put(upsert_stop).delete(delete_stop))
            .route("/routes", get(list_routes).post(create_route))
            .route("/routes/{id}", get(get_route))
            .route("/stop-times", get(list_stop_times).post(create_stop_time))
            .route("/stop-times/{id}", get(get_stop_time));
        omnia_wasi_http::serve(router, request).await
    }
}

// ---------------------------------------------------------------------------
// Stops
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Stop {
    stop_name: String,
    stop_lat: f64,
    stop_lon: f64,
    zone_id: Option<String>,
    wheelchair_boarding: Option<i32>,
    location_type: Option<i32>,
    parent_station: Option<String>,
    last_updated: Option<String>,
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
    exclude_zone: Option<String>,
    accessible: Option<bool>,
    top_level: Option<bool>,
    min_lat: Option<f64>,
    max_lat: Option<f64>,
    min_lon: Option<f64>,
    max_lon: Option<f64>,
    updated_on: Option<String>,
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
    if let Some(zone) = &p.exclude_zone {
        filters.push(Filter::ne("zone_id", zone.as_str()));
    }
    if p.accessible.unwrap_or(false) {
        filters.push(Filter::eq("wheelchair_boarding", 1));
        filters.push(Filter::is_not_null("zone_id"));
    }
    if p.top_level.unwrap_or(false) {
        filters.push(Filter::is_null("parent_station"));
    }
    if let Some(v) = p.min_lat {
        filters.push(Filter::gte("stop_lat", v));
    }
    if let Some(v) = p.max_lat {
        filters.push(Filter::lte("stop_lat", v));
    }
    if let Some(v) = p.min_lon {
        filters.push(Filter::gte("stop_lon", v));
    }
    if let Some(v) = p.max_lon {
        filters.push(Filter::lte("stop_lon", v));
    }
    if let Some(date) = &p.updated_on {
        filters.push(Filter::on_date("last_updated", date)?);
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

    let stops = deserialize_docs(&result.documents)?;
    Ok(Json(json!({ "stops": stops, "continuation": result.continuation })))
}

// ---------------------------------------------------------------------------
// Routes
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Route {
    agency_id: String,
    route_short_name: String,
    route_long_name: String,
    route_type: i32,
    route_color: Option<String>,
}

#[derive(Debug, Deserialize)]
struct CreateRouteRequest {
    id: String,
    #[serde(flatten)]
    route: Route,
}

#[derive(Debug, Deserialize)]
struct RouteQuery {
    q: Option<String>,
    types: Option<String>,
    agency: Option<String>,
    exclude_type: Option<i32>,
    not_agency: Option<String>,
    not_type: Option<i32>,
    limit: Option<u32>,
    continuation: Option<String>,
}

#[axum::debug_handler]
#[omnia_wasi_otel::instrument]
async fn create_route(Json(req): Json<CreateRouteRequest>) -> HttpResult<Json<Value>> {
    let doc = Document {
        id: req.id.clone(),
        data: serde_json::to_vec(&req.route).context("serializing route")?,
    };
    Provider.insert("routes", &doc).await.context("inserting route")?;
    Ok(Json(json!({ "route": req.route, "id": req.id })))
}

#[axum::debug_handler]
#[omnia_wasi_otel::instrument]
async fn get_route(Path(id): Path<String>) -> HttpResult<Json<Value>> {
    let doc = Provider
        .get("routes", &id)
        .await
        .context("fetching route")?
        .ok_or_else(|| anyhow!("route not found"))?;
    let route: Route = serde_json::from_slice(&doc.data).context("deserializing route")?;
    Ok(Json(json!({ "id": doc.id, "route": route })))
}

#[axum::debug_handler]
#[omnia_wasi_otel::instrument]
async fn list_routes(Query(p): Query<RouteQuery>) -> HttpResult<Json<Value>> {
    let mut filters = Vec::new();

    if let Some(q) = &p.q {
        filters.push(Filter::or([
            Filter::contains("route_short_name", q),
            Filter::contains("route_long_name", q),
        ]));
    }
    if let Some(types_str) = &p.types {
        let type_vals: Vec<ScalarValue> = types_str
            .split(',')
            .filter_map(|s| s.trim().parse::<i32>().ok())
            .map(ScalarValue::from)
            .collect();
        if !type_vals.is_empty() {
            filters.push(Filter::in_list("route_type", type_vals));
        }
    }
    if let Some(agency) = &p.agency {
        filters.push(Filter::eq("agency_id", agency.as_str()));
    }
    if let Some(exclude) = p.exclude_type {
        filters.push(Filter::not(Filter::eq("route_type", exclude)));
    }
    if let (Some(agency), Some(rtype)) = (&p.not_agency, p.not_type) {
        filters.push(Filter::not(Filter::and([
            Filter::eq("agency_id", agency.as_str()),
            Filter::eq("route_type", rtype),
        ])));
    }

    let filter = if filters.is_empty() { None } else { Some(Filter::and(filters)) };

    let result = Provider
        .query(
            "routes",
            QueryOptions {
                filter,
                order_by: vec![SortField {
                    field: "route_short_name".into(),
                    descending: false,
                }],
                limit: p.limit,
                continuation: p.continuation,
                ..Default::default()
            },
        )
        .await
        .context("querying routes")?;

    let routes = deserialize_docs(&result.documents)?;
    Ok(Json(json!({ "routes": routes, "continuation": result.continuation })))
}

// ---------------------------------------------------------------------------
// Stop times
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
struct StopTime {
    trip_id: String,
    stop_id: String,
    arrival_time: String,
    departure_time: String,
    stop_sequence: i32,
    pickup_type: Option<i32>,
    drop_off_type: Option<i32>,
}

#[derive(Debug, Deserialize)]
struct CreateStopTimeRequest {
    id: String,
    #[serde(flatten)]
    stop_time: StopTime,
}

#[derive(Debug, Deserialize)]
struct StopTimeQuery {
    trip: Option<String>,
    stop: Option<String>,
    after: Option<String>,
    before: Option<String>,
    min_seq: Option<i32>,
    max_seq: Option<i32>,
    limit: Option<u32>,
    continuation: Option<String>,
}

#[axum::debug_handler]
#[omnia_wasi_otel::instrument]
async fn create_stop_time(Json(req): Json<CreateStopTimeRequest>) -> HttpResult<Json<Value>> {
    let doc = Document {
        id: req.id.clone(),
        data: serde_json::to_vec(&req.stop_time).context("serializing stop_time")?,
    };
    Provider.insert("stop_times", &doc).await.context("inserting stop_time")?;
    Ok(Json(json!({ "stop_time": req.stop_time, "id": req.id })))
}

#[axum::debug_handler]
#[omnia_wasi_otel::instrument]
async fn get_stop_time(Path(id): Path<String>) -> HttpResult<Json<Value>> {
    let doc = Provider
        .get("stop_times", &id)
        .await
        .context("fetching stop_time")?
        .ok_or_else(|| anyhow!("stop_time not found"))?;
    let st: StopTime = serde_json::from_slice(&doc.data).context("deserializing stop_time")?;
    Ok(Json(json!({ "id": doc.id, "stop_time": st })))
}

#[axum::debug_handler]
#[omnia_wasi_otel::instrument]
async fn list_stop_times(Query(p): Query<StopTimeQuery>) -> HttpResult<Json<Value>> {
    let mut filters = Vec::new();

    if let Some(trip) = &p.trip {
        filters.push(Filter::eq("trip_id", trip.as_str()));
    }
    if let Some(stop) = &p.stop {
        filters.push(Filter::eq("stop_id", stop.as_str()));
    }
    if let Some(after) = &p.after {
        filters.push(Filter::gte("arrival_time", after.as_str()));
    }
    if let Some(before) = &p.before {
        filters.push(Filter::lte("arrival_time", before.as_str()));
    }
    if let Some(v) = p.min_seq {
        filters.push(Filter::gte("stop_sequence", v));
    }
    if let Some(v) = p.max_seq {
        filters.push(Filter::lte("stop_sequence", v));
    }

    let filter = if filters.is_empty() { None } else { Some(Filter::and(filters)) };

    let result = Provider
        .query(
            "stop_times",
            QueryOptions {
                filter,
                order_by: vec![SortField {
                    field: "stop_sequence".into(),
                    descending: false,
                }],
                limit: p.limit,
                continuation: p.continuation,
                ..Default::default()
            },
        )
        .await
        .context("querying stop_times")?;

    let stop_times = deserialize_docs(&result.documents)?;
    Ok(Json(json!({ "stop_times": stop_times, "continuation": result.continuation })))
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn deserialize_docs(docs: &[Document]) -> Result<Vec<Value>> {
    docs.iter()
        .map(|doc| {
            let mut val: Value =
                serde_json::from_slice(&doc.data).context("deserializing document")?;
            if let Value::Object(ref mut m) = val {
                m.insert("id".to_string(), Value::String(doc.id.clone()));
            }
            Ok(val)
        })
        .collect()
}

struct Provider;

impl DocumentStore for Provider {}
