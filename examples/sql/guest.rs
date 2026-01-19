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
//! - Executing SELECT queries with JOINs
//! - Executing INSERT/UPDATE/DELETE commands
//! - Foreign key relationships
//! - Converting results to JSON
//!
//! ## Security
//!
//! Uses parameterized queries (`$1`, `$2`, etc.) to prevent SQL injection.

#![cfg(target_arch = "wasm32")]

use anyhow::{Context, Result, anyhow};
use axum::extract::Path;
use axum::routing::{delete, get};
use axum::{Json, Router};
use chrono::Utc;
use qwasr_sdk::{HttpResult, TableStore};
use qwasr_wasi_sql::orm::{
    DeleteBuilder, Filter, InsertBuilder, Join, SelectBuilder, UpdateBuilder,
};
use qwasr_wasi_sql::types::{Connection, Statement};
use qwasr_wasi_sql::{entity, readwrite};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tracing::Level;
use wasip3::exports::http::handler::Guest;
use wasip3::http::types::{ErrorCode, Request, Response};

struct Http;
wasip3::http::proxy::export!(Http);

impl Guest for Http {
    /// Routes HTTP requests to database operations.
    #[qwasr_wasi_otel::instrument(name = "http_guest_handle", level = Level::DEBUG)]
    async fn handle(request: Request) -> Result<Response, ErrorCode> {
        tracing::debug!("received request: {:?}", request);
        let router = Router::new()
            .route("/agencies", get(list_agencies).post(create_agency))
            .route("/agencies/{id}", get(get_agency).patch(update_agency))
            .route("/agencies/{id}/feeds", get(list_agency_feeds).post(create_feed))
            .route("/feeds", get(list_all_feeds))
            .route("/feeds/{id}", delete(delete_feed));
        qwasr_wasi_http::serve(router, request).await
    }
}

/// List all agencies.
#[axum::debug_handler]
#[qwasr_wasi_otel::instrument]
async fn list_agencies() -> HttpResult<Json<Value>> {
    tracing::info!("list all agencies");
    ensure_schema().await?;

    let agencies = SelectBuilder::<Agency>::new()
        .order_by_desc(None, "created_at")
        .fetch(&Provider, "db")
        .await
        .context("failed to fetch agencies")?;

    Ok(Json(json!(agencies)))
}

/// Create a new agency.
#[axum::debug_handler]
#[qwasr_wasi_otel::instrument]
async fn create_agency(Json(req): Json<CreateAgencyRequest>) -> HttpResult<Json<Value>> {
    tracing::info!("create agency");
    ensure_schema().await?;

    let agencies = SelectBuilder::<Agency>::new()
        .order_by_desc(None, "agency_id")
        .limit(1)
        .fetch(&Provider, "db")
        .await
        .context("failed to fetch max agency_id")?;

    let next_id = agencies.first().map(|a| a.agency_id + 1).unwrap_or(1);

    let agency = Agency {
        agency_id: next_id,
        name: req.name,
        url: req.url,
        timezone: req.timezone,
        created_at: Utc::now().format("%Y-%m-%d %H:%M:%S").to_string(),
    };

    let query = InsertBuilder::<Agency>::from_entity(&agency)
        .build()
        .context("failed to build insert query")?;

    Provider
        .exec("db".to_string(), query.sql, query.params)
        .await
        .context("failed to insert agency")?;

    Ok(Json(json!({ "agency": agency })))
}

/// Get a specific agency.
#[axum::debug_handler]
#[qwasr_wasi_otel::instrument]
async fn get_agency(Path(id): Path<i64>) -> HttpResult<Json<Value>> {
    tracing::info!("get agency {}", id);
    ensure_schema().await?;

    let agencies = SelectBuilder::<Agency>::new()
        .r#where(Filter::eq("agency_id", id))
        .fetch(&Provider, "db")
        .await
        .context("failed to fetch agency")?;

    let agency = agencies.first().ok_or_else(|| anyhow!("agency not found"))?;

    Ok(Json(json!({ "agency": agency })))
}

/// Update an agency.
#[axum::debug_handler]
#[qwasr_wasi_otel::instrument]
async fn update_agency(
    Path(id): Path<i64>, Json(req): Json<UpdateAgencyRequest>,
) -> HttpResult<Json<Value>> {
    tracing::info!("update agency {}", id);
    ensure_schema().await?;

    // Verify agency exists
    let agencies = SelectBuilder::<Agency>::new()
        .r#where(Filter::eq("agency_id", id))
        .fetch(&Provider, "db")
        .await
        .context("failed to verify agency")?;

    let _ = agencies.first().ok_or_else(|| anyhow!("agency not found"))?;

    // Build update query - conditionally set only provided fields
    let mut update = UpdateBuilder::<Agency>::new();

    if let Some(name) = req.name {
        update = update.set("name", name);
    }
    if let Some(url) = req.url {
        update = update.set("url", url);
    }
    if let Some(timezone) = req.timezone {
        update = update.set("timezone", timezone);
    }

    let query = update
        .r#where(Filter::eq("agency_id", id))
        .build()
        .context("failed to build update query")?;

    Provider
        .exec("db".to_string(), query.sql, query.params)
        .await
        .context("failed to update agency")?;

    // Fetch updated agency
    let agencies = SelectBuilder::<Agency>::new()
        .r#where(Filter::eq("agency_id", id))
        .fetch(&Provider, "db")
        .await
        .context("failed to fetch updated agency")?;

    let agency = agencies.first().ok_or_else(|| anyhow!("agency not found after update"))?;

    Ok(Json(json!({ "agency": agency })))
}

/// List all feeds for a specific agency.
#[axum::debug_handler]
#[qwasr_wasi_otel::instrument]
async fn list_agency_feeds(Path(agency_id): Path<i64>) -> HttpResult<Json<Value>> {
    tracing::info!("list feeds for agency {}", agency_id);
    ensure_schema().await?;

    let feeds = SelectBuilder::<Feed>::new()
        .r#where(Filter::eq("agency_id", agency_id))
        .order_by_desc(None, "created_at")
        .fetch(&Provider, "db")
        .await
        .context("failed to fetch feeds")?;

    Ok(Json(json!({ "feeds": feeds })))
}

/// Create a new feed for an agency.
#[axum::debug_handler]
#[qwasr_wasi_otel::instrument]
async fn create_feed(
    Path(agency_id): Path<i64>, Json(req): Json<CreateFeedRequest>,
) -> HttpResult<Json<Value>> {
    tracing::info!("create feed for agency {}", agency_id);
    ensure_schema().await?;

    // Verify agency exists
    let agencies = SelectBuilder::<Agency>::new()
        .r#where(Filter::eq("agency_id", agency_id))
        .fetch(&Provider, "db")
        .await
        .context("failed to verify agency")?;

    if agencies.is_empty() {
        return Err(anyhow!("agency not found").into());
    }

    let feeds = SelectBuilder::<Feed>::new()
        .order_by_desc(None, "feed_id")
        .limit(1)
        .fetch(&Provider, "db")
        .await
        .context("failed to fetch max feed_id")?;

    let next_id = feeds.first().map(|f| f.feed_id + 1).unwrap_or(1);

    let feed = Feed {
        feed_id: next_id,
        agency_id,
        description: req.description,
        created_at: Utc::now().format("%Y-%m-%d %H:%M:%S").to_string(),
    };

    let query = InsertBuilder::<Feed>::from_entity(&feed)
        .build()
        .context("failed to build insert query")?;

    Provider
        .exec("db".to_string(), query.sql, query.params)
        .await
        .context("failed to insert feed")?;

    Ok(Json(json!({ "feed": feed })))
}

/// List all feeds with their agency information (demonstrates JOIN).
#[axum::debug_handler]
#[qwasr_wasi_otel::instrument]
async fn list_all_feeds() -> HttpResult<Json<Value>> {
    tracing::info!("list all feeds with agency info");
    ensure_schema().await?;

    let feeds_with_agency = SelectBuilder::<FeedWithAgency>::new()
        .order_by_desc(Some("feed"), "created_at")
        .limit(100)
        .fetch(&Provider, "db")
        .await
        .context("failed to fetch feeds with agencies")?;

    Ok(Json(json!({ "feeds": feeds_with_agency })))
}

/// Delete a specific feed.
#[axum::debug_handler]
#[qwasr_wasi_otel::instrument]
async fn delete_feed(Path(id): Path<i64>) -> HttpResult<Json<Value>> {
    tracing::info!("delete feed {}", id);
    ensure_schema().await?;

    let query = DeleteBuilder::<Feed>::new()
        .r#where(Filter::eq("feed_id", id))
        .build()
        .context("failed to build delete query")?;

    let rows_affected = Provider
        .exec("db".to_string(), query.sql, query.params)
        .await
        .context("failed to delete feed")?;

    if rows_affected == 0 {
        return Err(anyhow!("feed not found").into());
    }

    Ok(Json(json!({ "message": "feed deleted", "feed_id": id })))
}

async fn ensure_schema() -> Result<()> {
    let pool = Connection::open("db".to_string())
        .await
        .map_err(|e| anyhow!("failed to open connection: {}", e.trace()))?;

    // Create agency table
    let create_agency = "CREATE TABLE IF NOT EXISTS agency (
        agency_id INTEGER PRIMARY KEY,
        name TEXT NOT NULL,
        url TEXT,
        timezone TEXT,
        created_at TEXT NOT NULL
    )";

    let stmt = Statement::prepare(create_agency.to_string(), vec![])
        .await
        .map_err(|e| anyhow!("failed to prepare agency table creation: {}", e.trace()))?;

    readwrite::exec(&pool, &stmt)
        .await
        .map_err(|e| anyhow!("agency table creation failed: {}", e.trace()))?;

    // Create feed table
    let create_feed = "CREATE TABLE IF NOT EXISTS feed (
        feed_id INTEGER PRIMARY KEY,
        agency_id INTEGER NOT NULL,
        description TEXT NOT NULL,
        created_at TEXT NOT NULL
    )";

    let stmt = Statement::prepare(create_feed.to_string(), vec![])
        .await
        .map_err(|e| anyhow!("failed to prepare feed table creation: {}", e.trace()))?;

    readwrite::exec(&pool, &stmt)
        .await
        .map_err(|e| anyhow!("feed table creation failed: {}", e.trace()))?;

    tracing::debug!("Schema initialized!");
    Ok(())
}

// Entity definitions

entity!(
    table = "agency",
    #[derive(Debug, Clone, Serialize)]
    pub struct Agency {
        pub agency_id: i64,
        pub name: String,
        pub url: Option<String>,
        pub timezone: Option<String>,
        pub created_at: String,
    }
);

entity!(
    table = "feed",
    #[derive(Debug, Clone, Serialize)]
    pub struct Feed {
        pub feed_id: i64,
        pub agency_id: i64,
        pub description: String,
        pub created_at: String,
    }
);

// Entity with JOIN - demonstrates the power of joins
// Uses the `columns` parameter to manually specify columns from the joined agency table.
// Fields not in `columns` are auto-qualified with the main table (feed).
entity!(
    table = "feed",
    columns = [
        ("agency", "name", "agency_name"),
        ("agency", "url", "agency_url"),
        ("agency", "timezone", "agency_timezone"),
    ],
    joins = [Join::left("agency", Filter::col_eq("feed", "agency_id", "agency", "agency_id")),],
    #[derive(Debug, Clone, Serialize)]
    pub struct FeedWithAgency {
        pub feed_id: i64,                    // Auto: feed.feed_id
        pub agency_id: i64,                  // Auto: feed.agency_id
        pub description: String,             // Auto: feed.description
        pub created_at: String,              // Auto: feed.created_at
        pub agency_name: String,             // Manual: agency.name AS agency_name
        pub agency_url: Option<String>,      // Manual: agency.url AS agency_url
        pub agency_timezone: Option<String>, // Manual: agency.timezone AS agency_timezone
    }
);

// Request types

#[derive(Debug, Deserialize)]
struct CreateAgencyRequest {
    name: String,
    url: Option<String>,
    timezone: Option<String>,
}

#[derive(Debug, Deserialize)]
struct UpdateAgencyRequest {
    name: Option<String>,
    url: Option<String>,
    timezone: Option<String>,
}

#[derive(Debug, Deserialize)]
struct CreateFeedRequest {
    description: String,
}

struct Provider;

impl TableStore for Provider {}
