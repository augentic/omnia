//! # SQL Wasm Guest (Default Backend)
//!
//! Demonstrates the WASI SQL interface: opening connections, preparing
//! parameterized statements (`$1`, `$2`, ...) for schema creation, and
//! running hand-written CRUD SQL through the `TableStore` capability with
//! row-to-struct mapping over `Row` fields.

#![cfg(target_arch = "wasm32")]

use anyhow::{Context, Result, anyhow, bail};
use axum::extract::Path;
use axum::routing::{delete, get};
use axum::{Json, Router};
use chrono::Utc;
use omnia_sdk::{HttpResult, TableStore};
use omnia_wasi_sql::types::{Connection, Statement};
use omnia_wasi_sql::{DataType, Row, readwrite};
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

/// List all agencies, newest first (SELECT).
#[axum::debug_handler]
#[omnia_wasi_otel::instrument]
async fn list_agencies() -> HttpResult<Json<Value>> {
    ensure_schema().await?;

    let rows = Provider
        .query(
            "db".to_string(),
            "SELECT agency_id, name, url, timezone, created_at FROM agency ORDER BY created_at DESC"
                .to_string(),
            vec![],
        )
        .await
        .context("executing query")?;

    let agencies =
        rows.iter().map(Agency::from_row).collect::<Result<Vec<_>>>().context("mapping rows")?;

    Ok(Json(json!({ "agencies": agencies })))
}

/// Create an agency with a client-supplied id (INSERT).
#[axum::debug_handler]
#[omnia_wasi_otel::instrument]
async fn create_agency(Json(req): Json<CreateAgencyRequest>) -> HttpResult<Json<Value>> {
    ensure_schema().await?;

    let agency = Agency {
        agency_id: req.agency_id,
        name: req.name,
        url: req.url,
        timezone: req.timezone,
        created_at: Utc::now().format("%Y-%m-%d %H:%M:%S").to_string(),
    };

    Provider
        .exec(
            "db".to_string(),
            "INSERT INTO agency (agency_id, name, url, timezone, created_at) \
             VALUES ($1, $2, $3, $4, $5)"
                .to_string(),
            vec![
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

/// Delete an agency (DELETE); zero affected rows means it did not exist.
#[axum::debug_handler]
#[omnia_wasi_otel::instrument]
async fn delete_agency(Path(id): Path<i64>) -> HttpResult<Json<Value>> {
    ensure_schema().await?;

    let rows_affected = Provider
        .exec(
            "db".to_string(),
            "DELETE FROM agency WHERE agency_id = $1".to_string(),
            vec![DataType::Int64(Some(id))],
        )
        .await
        .context("deleting agency")?;

    if rows_affected == 0 {
        return Err(anyhow!("agency not found").into());
    }

    Ok(Json(json!({ "message": "agency deleted", "agency_id": id })))
}

/// Create the schema with raw prepared statements. Each request is handled by
/// a fresh guest instance, so this runs per request — fine for an example.
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

#[derive(Debug, Clone, Serialize)]
struct Agency {
    agency_id: i64,
    name: String,
    url: Option<String>,
    timezone: Option<String>,
    created_at: String,
}

impl Agency {
    fn from_row(row: &Row) -> Result<Self> {
        Ok(Self {
            agency_id: int(field(row, "agency_id")?)?,
            name: text(field(row, "name")?)?,
            url: nullable_text(field(row, "url")?)?,
            timezone: nullable_text(field(row, "timezone")?)?,
            created_at: text(field(row, "created_at")?)?,
        })
    }
}

fn field<'a>(row: &'a Row, name: &str) -> Result<&'a DataType> {
    row.fields
        .iter()
        .find(|f| f.name == name)
        .map(|f| &f.value)
        .ok_or_else(|| anyhow!("missing column `{name}`"))
}

fn int(value: &DataType) -> Result<i64> {
    match value {
        DataType::Int64(Some(v)) => Ok(*v),
        DataType::Int32(Some(v)) => Ok(i64::from(*v)),
        other => bail!("expected integer, got {other:?}"),
    }
}

fn text(value: &DataType) -> Result<String> {
    nullable_text(value)?.ok_or_else(|| anyhow!("unexpected NULL"))
}

// The default backend reports NULL as `Str(None)` regardless of column type.
fn nullable_text(value: &DataType) -> Result<Option<String>> {
    match value {
        DataType::Str(v) => Ok(v.clone()),
        other => bail!("expected text, got {other:?}"),
    }
}

// Request types

#[derive(Debug, Deserialize)]
struct CreateAgencyRequest {
    agency_id: i64,
    name: String,
    url: Option<String>,
    timezone: Option<String>,
}

struct Provider;

impl TableStore for Provider {}
