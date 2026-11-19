//! # SQL Wasm Guest (Default Backend)
//!
//! Demonstrates `omnia:sql`: opening the default database and creating the
//! schema, then running hand-written CRUD SQL with `$1`, `$2`, ...
//! placeholders through the `Database` helpers, with rows decoded into a
//! record type through `FromRow`.

#![cfg(target_arch = "wasm32")]

use anyhow::{Context, Result, anyhow};
use axum::extract::Path;
use axum::routing::{delete, get};
use axum::{Json, Router};
use chrono::Utc;
use omnia_sdk::sql::{self, Database, FromRow, Value};
use serde::{Deserialize, Serialize};
use serde_json::json;
use wasip3::exports::http::handler::Guest;
use wasip3::http::types::{ErrorCode, Request, Response};

struct Http;
wasip3::http::service::export!(Http);

impl Guest for Http {
    async fn handle(request: Request) -> Result<Response, ErrorCode> {
        let router = Router::new()
            .route("/agencies", get(list_agencies).post(create_agency))
            .route("/agencies/{id}", delete(delete_agency));
        omnia_sdk::http::serve(router, request).await
    }
}

#[axum::debug_handler]
#[omnia_sdk::otel::instrument]
async fn list_agencies() -> omnia_sdk::Result<Json<serde_json::Value>> {
    let database = database().await?;

    let agencies: Vec<Agency> = database
        .query_as(
            "SELECT agency_id, name, url, timezone, created_at FROM agency ORDER BY created_at DESC",
            &[],
        )
        .await
        .context("executing query")?;

    Ok(Json(json!({ "agencies": agencies })))
}

#[axum::debug_handler]
#[omnia_sdk::otel::instrument]
async fn create_agency(
    Json(req): Json<CreateAgencyRequest>,
) -> omnia_sdk::Result<Json<serde_json::Value>> {
    let database = database().await?;

    let agency = Agency {
        agency_id: req.agency_id,
        name: req.name,
        url: req.url,
        timezone: req.timezone,
        created_at: Utc::now().format("%Y-%m-%d %H:%M:%S").to_string(),
    };

    database
        .exec(
            "INSERT INTO agency (agency_id, name, url, timezone, created_at) \
             VALUES ($1, $2, $3, $4, $5)",
            &[
                Value::from(agency.agency_id),
                Value::from(agency.name.as_str()),
                Value::from(agency.url.as_deref()),
                Value::from(agency.timezone.as_deref()),
                Value::from(agency.created_at.as_str()),
            ],
        )
        .await
        .context("inserting agency")?;

    Ok(Json(json!({ "agency": agency })))
}

// zero affected rows means it did not exist
#[axum::debug_handler]
#[omnia_sdk::otel::instrument]
async fn delete_agency(Path(id): Path<i64>) -> omnia_sdk::Result<Json<serde_json::Value>> {
    let database = database().await?;

    let rows_affected = database
        .exec("DELETE FROM agency WHERE agency_id = $1", &[Value::from(id)])
        .await
        .context("deleting agency")?;

    if rows_affected == 0 {
        return Err(anyhow!("agency not found").into());
    }

    Ok(Json(json!({ "message": "agency deleted", "agency_id": id })))
}

// creates the schema per request, since each is handled by a fresh instance;
// fine for an example
async fn database() -> Result<Database> {
    let database = sql::open(sql::DEFAULT).await.context("opening the database")?;

    let create_agency = "CREATE TABLE IF NOT EXISTS agency (
        agency_id BIGINT PRIMARY KEY,
        name TEXT NOT NULL,
        url TEXT,
        timezone TEXT,
        created_at TEXT NOT NULL
    )";

    database.exec(create_agency, &[]).await.context("creating agency table")?;

    Ok(database)
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
