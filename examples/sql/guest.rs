//! # SQL Wasm Guest (Default Backend)
//!
//! This module demonstrates the WASI SQL interface with the default backend.
//! It shows how to perform database operations that work with any SQL-compatible
//! database configured by the host.
//!
//! ## Operations Demonstrated
//!
//! - Opening database connections by name
//! - Preparing parameterized SQL statements
//! - Executing SELECT queries
//! - Executing INSERT/UPDATE/DELETE commands
//! - Converting results to JSON
//!
//! ## Security
//!
//! Always use parameterized queries (`$1`, `$2`, etc.) to prevent SQL injection.
//! Never concatenate user input into SQL strings.
//!
//! ## Backend Agnostic
//!
//! This guest code works with any WASI SQL backend:
//! - PostgreSQL (sql-postgres example)
//! - Azure SQL
//! - Any SQL-compatible database

#![cfg(target_arch = "wasm32")]

use anyhow::anyhow;
use axum::routing::{get, post};
use axum::{Json, Router};
use bytes::Bytes;
use serde_json::{Value, json};
use tracing::Level;
use yetti_sdk::HttpResult;
use wasi_sql::types::{Connection, DataType, FormattedValue, Statement};
use wasi_sql::{into_json, readwrite};
use wasip3::exports::http::handler::Guest;
use wasip3::http::types::{ErrorCode, Request, Response};

struct Http;
wasip3::http::proxy::export!(Http);

impl Guest for Http {
    /// Routes HTTP requests to database operations.
    #[wasi_otel::instrument(name = "http_guest_handle", level = Level::DEBUG)]
    async fn handle(request: Request) -> Result<Response, ErrorCode> {
        tracing::debug!("received request: {:?}", request);
        let router = Router::new().route("/", get(query)).route("/", post(insert));
        wasi_http::serve(router, request).await
    }
}

/// Queries all rows from the sample table.
#[axum::debug_handler]
#[wasi_otel::instrument]
async fn query() -> HttpResult<Json<Value>> {
    tracing::info!("query database");

    let pool = Connection::open("postgres".to_string())
        .await
        .map_err(|e| anyhow!("failed to open connection: {e:?}"))?;

    let stmt = Statement::prepare("SELECT * from mytable;".to_string(), vec![])
        .await
        .map_err(|e| anyhow!("failed to prepare statement: {e:?}"))?;

    let res = readwrite::query(&pool, &stmt).await.map_err(|e| anyhow!("query failed: {e:?}"))?;

    Ok(Json(into_json(res)?))
}

/// Inserts a new row into the sample table.
#[axum::debug_handler]
#[wasi_otel::instrument]
async fn insert(_body: Bytes) -> HttpResult<Json<Value>> {
    tracing::info!("insert data");

    let insert = "insert into mytable (feed_id, agency_id, agency_name, agency_url, agency_timezone, created_at) values ($1, $2, $3, $4, $5, $6);";

    let params: Vec<DataType> = [
        DataType::Int32(Some(1224)),
        DataType::Str(Some("test1".to_string())),
        DataType::Str(Some("name1".to_string())),
        DataType::Str(Some("url1".to_string())),
        DataType::Str(Some("NZL".to_string())),
        DataType::Timestamp(Some(FormattedValue {
            value: "2025-11-06T00:05:30".to_string(),
            format: "%Y-%m-%dT%H:%M:%S".to_string(),
        })),
    ]
    .to_vec();

    tracing::debug!("opening connection");

    let pool = Connection::open("db".to_string())
        .await
        .map_err(|e| anyhow!("failed to open connection: {e:?}"))?;
    let stmt = Statement::prepare(insert.to_string(), params)
        .await
        .map_err(|e| anyhow!("failed to prepare statement: {e:?}"))?;

    let res = readwrite::exec(&pool, &stmt).await.map_err(|e| anyhow!("query failed: {e:?}"))?;

    Ok(Json(json!({
        "message": res
    })))
}
