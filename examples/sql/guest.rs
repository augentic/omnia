//! # SQL Wasm Guest (Default Backend)
//!
//! Demonstrates the WASI SQL interface: opening connections, preparing
//! parameterized statements, and the guest ORM — `SelectBuilder`,
//! `InsertBuilder`, `UpdateBuilder`, `DeleteBuilder`, and an `entity!`
//! JOIN mapping. Uses parameterized queries (`$1`, `$2`, ...) throughout.

#![cfg(target_arch = "wasm32")]

use anyhow::{Context, Result, anyhow};
use axum::extract::Path;
use axum::routing::{delete, get, patch, post};
use axum::{Json, Router};
use chrono::Utc;
use omnia_guest::orm::{
    DeleteBuilder, Entity, Filter, InsertBuilder, Join, SelectBuilder, UpdateBuilder,
};
use omnia_guest::{HttpResult, TableStore, entity};
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
            .route("/agencies/{id}", patch(update_agency))
            .route("/agencies/{id}/feeds", post(create_feed))
            .route("/feeds", get(list_all_feeds))
            .route("/feeds/{id}", delete(delete_feed));
        omnia_wasi_http::serve(router, request).await
    }
}

/// List all agencies (`SelectBuilder`).
#[axum::debug_handler]
#[omnia_wasi_otel::instrument]
async fn list_agencies() -> HttpResult<Json<Value>> {
    ensure_schema().await?;

    let select = SelectBuilder::<Agency>::new()
        .order_by_desc(None, "created_at")
        .build()
        .context("building query")?;

    let rows = Provider
        .query("db".to_string(), select.sql, select.params)
        .await
        .context("executing query")?;

    let agencies =
        rows.iter().map(Agency::from_row).collect::<Result<Vec<_>>>().context("mapping rows")?;

    Ok(Json(json!({ "agencies": agencies })))
}

/// Create an agency with a client-supplied id (`InsertBuilder`).
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

    let query = InsertBuilder::<Agency>::from_entity(&agency).build().context("building insert")?;

    Provider.exec("db".to_string(), query.sql, query.params).await.context("inserting agency")?;

    Ok(Json(json!({ "agency": agency })))
}

/// Update an agency's mutable fields (`UpdateBuilder`).
#[axum::debug_handler]
#[omnia_wasi_otel::instrument]
async fn update_agency(
    Path(id): Path<i64>, Json(req): Json<UpdateAgencyRequest>,
) -> HttpResult<Json<Value>> {
    ensure_schema().await?;

    let query = UpdateBuilder::<Agency>::new()
        .set("name", req.name)
        .set("timezone", req.timezone)
        .r#where(Filter::eq("agency_id", id))
        .build()
        .context("building update")?;

    let rows_affected = Provider
        .exec("db".to_string(), query.sql, query.params)
        .await
        .context("updating agency")?;

    if rows_affected == 0 {
        return Err(anyhow!("agency not found").into());
    }

    Ok(Json(json!({ "message": "agency updated", "agency_id": id })))
}

/// Create a feed for an existing agency (`InsertBuilder` + existence check).
#[axum::debug_handler]
#[omnia_wasi_otel::instrument]
async fn create_feed(
    Path(agency_id): Path<i64>, Json(req): Json<CreateFeedRequest>,
) -> HttpResult<Json<Value>> {
    ensure_schema().await?;

    let select = SelectBuilder::<Agency>::new()
        .r#where(Filter::eq("agency_id", agency_id))
        .limit(1)
        .build()
        .context("building agency lookup")?;

    let rows = Provider
        .query("db".to_string(), select.sql, select.params)
        .await
        .context("executing agency lookup")?;

    if rows.is_empty() {
        return Err(anyhow!("agency not found").into());
    }

    let feed = Feed {
        feed_id: req.feed_id,
        agency_id,
        description: req.description,
        created_at: Utc::now().format("%Y-%m-%d %H:%M:%S").to_string(),
    };

    let query = InsertBuilder::<Feed>::from_entity(&feed).build().context("building insert")?;

    Provider.exec("db".to_string(), query.sql, query.params).await.context("inserting feed")?;

    Ok(Json(json!({ "feed": feed })))
}

/// List all feeds with their agency information (`entity!` JOIN).
#[axum::debug_handler]
#[omnia_wasi_otel::instrument]
async fn list_all_feeds() -> HttpResult<Json<Value>> {
    ensure_schema().await?;

    let select = SelectBuilder::<FeedWithAgency>::new()
        .order_by_desc(Some("feed"), "created_at")
        .limit(100)
        .build()
        .context("building join query")?;

    let rows = Provider
        .query("db".to_string(), select.sql, select.params)
        .await
        .context("executing join query")?;

    let feeds = rows
        .iter()
        .map(FeedWithAgency::from_row)
        .collect::<Result<Vec<_>>>()
        .context("mapping rows")?;

    Ok(Json(json!({ "feeds": feeds })))
}

/// Delete a feed (`DeleteBuilder`).
#[axum::debug_handler]
#[omnia_wasi_otel::instrument]
async fn delete_feed(Path(id): Path<i64>) -> HttpResult<Json<Value>> {
    ensure_schema().await?;

    let query = DeleteBuilder::<Feed>::new()
        .r#where(Filter::eq("feed_id", id))
        .build()
        .context("building delete")?;

    let rows_affected =
        Provider.exec("db".to_string(), query.sql, query.params).await.context("deleting feed")?;

    if rows_affected == 0 {
        return Err(anyhow!("feed not found").into());
    }

    Ok(Json(json!({ "message": "feed deleted", "feed_id": id })))
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

    let create_feed = "CREATE TABLE IF NOT EXISTS feed (
        feed_id INTEGER PRIMARY KEY,
        agency_id INTEGER NOT NULL,
        description TEXT NOT NULL,
        created_at TEXT NOT NULL
    )";

    let stmt = Statement::prepare(create_feed.to_string(), vec![])
        .await
        .map_err(|e| anyhow!("preparing feed table creation: {}", e.trace()))?;

    readwrite::exec(&pool, &stmt)
        .await
        .map_err(|e| anyhow!("creating feed table: {}", e.trace()))?;

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

// JOIN entity: `columns` names fields sourced from the joined agency table;
// fields not listed are auto-qualified with the main table (feed).
entity!(
    table = "feed",
    columns = [("agency", "name", "agency_name"),],
    joins = [Join::left("agency", Filter::col_eq("feed", "agency_id", "agency", "agency_id")),],
    #[derive(Debug, Clone, Serialize)]
    pub struct FeedWithAgency {
        pub feed_id: i64,
        pub agency_id: i64,
        pub description: String,
        pub created_at: String,
        pub agency_name: String,
    }
);

// Request types

#[derive(Debug, Deserialize)]
struct CreateAgencyRequest {
    agency_id: i64,
    name: String,
    url: Option<String>,
    timezone: Option<String>,
}

#[derive(Debug, Deserialize)]
struct UpdateAgencyRequest {
    name: String,
    timezone: String,
}

#[derive(Debug, Deserialize)]
struct CreateFeedRequest {
    feed_id: i64,
    description: String,
}

struct Provider;

impl TableStore for Provider {}
