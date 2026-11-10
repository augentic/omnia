//! # SQL Wasm Guest (Default Backend)
//!
//! Demonstrates the WASI SQL interface: opening connections, preparing
//! parameterized statements (`$1`, `$2`, ...) for schema creation, and
//! running hand-written CRUD SQL through the `TableStore` capability with
//! rows decoded into a record type through `FromRow`.

#![cfg(target_arch = "wasm32")]

use anyhow::{Context, Result, anyhow};
use axum::extract::Path;
use axum::routing::{delete, get};
use axum::{Json, Router};
use chrono::Utc;
use omnia_sdk::sql::{DataType, FromRow};
use omnia_sdk::{TableStore, TableStoreExt as _, WasiProvider};
use omnia_wasi_sql::readwrite;
use omnia_wasi_sql::types::{Connection, Statement};
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
            .route("/agencies", get(list_agencies).post(create_agency))
            .route("/agencies/{id}", delete(delete_agency));
        omnia_wasi_http::serve(router, request).await
    }
}

#[axum::debug_handler]
#[omnia_wasi_otel::instrument]
async fn list_agencies() -> omnia_sdk::Result<Json<Value>> {
    ensure_schema().await?;

    let agencies: Vec<Agency> = WasiProvider
        .query_as(
            "db",
            "SELECT agency_id, name, url, timezone, created_at FROM agency ORDER BY created_at DESC",
            &[],
        )
        .await
        .context("executing query")?;

    Ok(Json(json!({ "agencies": agencies })))
}

#[axum::debug_handler]
#[omnia_wasi_otel::instrument]
async fn create_agency(Json(req): Json<CreateAgencyRequest>) -> omnia_sdk::Result<Json<Value>> {
    ensure_schema().await?;

    let agency = Agency {
        agency_id: req.agency_id,
        name: req.name,
        url: req.url,
        timezone: req.timezone,
        created_at: Utc::now().format("%Y-%m-%d %H:%M:%S").to_string(),
    };

    WasiProvider
        .exec(
            "db",
            "INSERT INTO agency (agency_id, name, url, timezone, created_at) \
             VALUES ($1, $2, $3, $4, $5)",
            &[
                DataType::Int64(Some(agency.agency_id)),
                DataType::Str(Some(agency.name.clone())),
                DataType::Str(agency.url.clone()),
                DataType::Str(agency.timezone.clone()),
                DataType::Str(Some(agency.created_at.clone())),
            ],
        )
        .await
        .context("inserting agency")?;

    Ok(Json(json!({ "agency": agency })))
}

// zero affected rows means it did not exist
#[axum::debug_handler]
#[omnia_wasi_otel::instrument]
async fn delete_agency(Path(id): Path<i64>) -> omnia_sdk::Result<Json<Value>> {
    ensure_schema().await?;

    let rows_affected = WasiProvider
        .exec("db", "DELETE FROM agency WHERE agency_id = $1", &[DataType::Int64(Some(id))])
        .await
        .context("deleting agency")?;

    if rows_affected == 0 {
        return Err(anyhow!("agency not found").into());
    }

    Ok(Json(json!({ "message": "agency deleted", "agency_id": id })))
}

// runs per request, since each is handled by a fresh instance; fine for an example
async fn ensure_schema() -> Result<()> {
    let pool = Connection::open("db".to_string())
        .await
        .map_err(|e| anyhow!("opening connection: {}", e.trace()))?;

    let create_agency = "CREATE TABLE IF NOT EXISTS agency (
        agency_id INTEGER PRIMARY KEY,
        name TEXT NOT NULL,
        url TEXT,
        timezone TEXT,
        created_at TEXT NOT NULL
    )";

    let stmt = Statement::prepare(create_agency.to_string(), vec![])
        .await
        .map_err(|e| anyhow!("preparing agency table creation: {}", e.trace()))?;

    readwrite::exec(&pool, &stmt)
        .await
        .map_err(|e| anyhow!("creating agency table: {}", e.trace()))?;

    Ok(())
}

// Row mapping

#[derive(Debug, Clone, Serialize, FromRow)]
struct Agency {
    agency_id: i64,
    name: String,
    url: Option<String>,
    timezone: Option<String>,
    created_at: String,
}

// Request types

#[derive(Debug, Deserialize)]
struct CreateAgencyRequest {
    agency_id: i64,
    name: String,
    url: Option<String>,
    timezone: Option<String>,
}
