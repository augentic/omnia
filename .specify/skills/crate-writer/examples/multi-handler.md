# Multi Handler Example: cars

An API crate with multiple HTTP handlers for querying external APIs, featuring query string parsing, filter builders, and related data fetching. From the `traffic` project.

## Crate Structure

```
crates/cars/
├── Cargo.toml
├── src/
│   ├── lib.rs              # Module declarations, re-exports, constant
│   ├── filter.rs           # Query filter builder utility
│   ├── handlers.rs         # Barrel module + shared types
│   └── handlers/
│       ├── feature.rs      # Single feature by ID (String input)
│       ├── feature_list.rs # All features (unit input)
│       ├── layout.rs       # Layouts by TMP IDs (query string input)
│       └── worksite.rs     # Worksite by code (query string input)
├── tests/
│   ├── provider.rs         # MockProvider
│   ├── feature.rs          # Feature tests
│   ├── layout.rs           # Layout tests
│   └── worksite.rs         # Worksite tests
└── tests/data/             # JSON fixture files
```

## src/lib.rs

Minimal: module declarations, re-exports, and shared constants.

```rust
//! # Cars Integration Adapter
//!
//! Transforms CARs and TMP data sources into structured responses.

mod filter;
mod handlers;

pub use handlers::*;
pub use omnia_sdk::{Config, Error, HttpError, HttpRequest, Result};

pub const MWS_URI: &str = "https://api.myworksites.co.nz/v1/prod";
```

### Key Patterns

- Re-export SDK traits so tests can import from the crate directly
- Constants for API base URIs
- No error types in `lib.rs` when the crate uses only SDK error macros

## src/handlers.rs

Barrel module declaring sub-modules and shared types used across handlers.

```rust
//! Handlers for processing CARS-related requests.

mod feature;
mod feature_list;
mod layout;
mod worksite;

pub use feature::*;
pub use feature_list::*;
pub use layout::*;
use serde::{Deserialize, Serialize};
pub use worksite::*;

/// A worksite entry returned by the MyWorkSites API.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Worksite {
    pub worksite_id: u64,
    pub worksite_code: String,
    pub name: String,
    pub project_name: String,
    pub start_date: Option<String>,
    pub end_date: Option<String>,
    pub work_status: String,
    pub location: Location,
    pub tmps: Option<Vec<Tmp>>,
    // ... additional fields
}

/// GeoJSON geometry representation.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum Geometry {
    Point { coordinates: (f64, f64) },
    LineString { coordinates: Vec<(f64, f64)> },
    Polygon { coordinates: Vec<Vec<(f64, f64)>> },
    MultiPolygon { coordinates: Vec<Vec<Vec<(f64, f64)>>> },
    GeometryCollection { geometries: Vec<Self> },
}

impl Default for Geometry {
    fn default() -> Self {
        Self::Polygon { coordinates: vec![] }
    }
}
```

### Key Patterns

- Barrel module with `pub use` for all sub-module exports
- Shared types (`Worksite`, `Geometry`) that multiple handlers reference
- `#[serde(rename_all = "camelCase", default)]` for API response types
- Tagged enum for polymorphic JSON (`#[serde(tag = "type")]`)
- Manual `Default` impl when automatic derivation is not possible

## src/handlers/feature.rs

Single feature lookup by ID. Demonstrates `type Input = String` for path parameter handlers.

```rust
//! Fetches a single CARS feature by worksite ID.

use anyhow::Context as _;
use bytes::Bytes;
use chrono::{DateTime, Utc};
use http_body_util::Empty;
use omnia_sdk::{Config, Context, Error, Handler, IntoBody, Reply, Result, bad_gateway};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::filter::{Cmp, Filter};
use crate::handlers::{Geometry, Worksite};
use crate::{HttpRequest, MWS_URI};

async fn handle<P>(_owner: &str, req: FeatureRequest, provider: &P) -> Result<FeatureResponse>
where
    P: Config + HttpRequest,
{
    let api_key = Config::get(provider, "MWS_API_KEY").await?;

    let filter = Filter::new()
        .condition("worksiteId", Cmp::Eq(json!(req.id)))
        .to_encoded();

    let request = http::Request::builder()
        .uri(format!("{MWS_URI}/worksite-search?filter={filter}"))
        .header("x-api-key", &api_key)
        .body(Empty::<Bytes>::new())
        .context("building request")?;
    let response = HttpRequest::fetch(provider, request)
        .await
        .map_err(|e| bad_gateway!("fetching worksites: {e}"))?;

    let bytes = response.into_body();
    let worksites: Vec<Worksite> =
        serde_json::from_slice(&bytes).context("deserializing worksites response")?;

    let worksite = worksites.first().cloned().ok_or_else(|| Error::NotFound {
        code: "not_found".to_string(),
        description: "feature not found".to_string(),
    })?;

    Ok(FeatureResponse(MwsFeature::from(worksite)))
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct FeatureRequest {
    pub id: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FeatureResponse(pub MwsFeature);

impl IntoBody for FeatureResponse {
    fn into_body(self) -> anyhow::Result<Vec<u8>> {
        serde_json::to_vec(&self).context("serializing reply")
    }
}

impl<P> Handler<P> for FeatureRequest
where
    P: Config + HttpRequest,
{
    type Error = Error;
    type Input = String;      // <-- single path parameter
    type Output = FeatureResponse;

    fn from_input(input: String) -> Result<Self> {
        Ok(Self { id: input })
    }

    async fn handle(self, ctx: Context<'_, P>) -> Result<Reply<FeatureResponse>> {
        Ok(handle(ctx.owner, self, ctx.provider).await?.into())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MwsFeature {
    #[serde(skip_deserializing)]
    pub id: String,
    pub geometry: Geometry,
    pub properties: MwsProperties,
}

impl From<Worksite> for MwsFeature {
    fn from(worksite: Worksite) -> Self {
        Self {
            id: worksite.worksite_id.to_string(),
            geometry: worksite.location.geometry,
            properties: MwsProperties {
                worksite_code: worksite.worksite_code,
                worksite_name: worksite.name,
                project_name: worksite.project_name,
                work_status: worksite.work_status,
                // ...
                ..MwsProperties::default()
            },
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MwsProperties {
    pub worksite_code: String,
    pub worksite_name: String,
    pub project_name: String,
    pub worksite_type: Option<String>,
    pub work_status: String,
    // ...
}
```

### Key Patterns

- `type Input = String` for single path parameter
- `IntoBody` implementation for HTTP response types
- `From<Worksite> for MwsFeature` conversion with struct update syntax
- `bad_gateway!` macro for upstream API failures
- `Error::NotFound` for missing resources
- `handle()` returns the domain response type; the `Handler` impl wraps it in `Reply` via `.into()`

## src/handlers/worksite.rs

Worksite lookup with query string parsing and optional related data. Demonstrates `type Input = Option<String>`.

```rust
//! Returns worksite details with optional TMPs.

use anyhow::Context as _;
use bytes::Bytes;
use chrono::NaiveDate;
use http_body_util::Empty;
use omnia_sdk::api::{Context, Handler, Reply};
use omnia_sdk::{Config, IntoBody, bad_gateway};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::filter::{Cmp, Filter};
use crate::handlers::Worksite;
use crate::{Error, HttpRequest, MWS_URI, Result};

async fn handle<P>(_owner: &str, req: WorksiteRequest, provider: &P) -> Result<WorksiteResponse>
where
    P: Config + HttpRequest,
{
    let api_key = Config::get(provider, "MWS_API_KEY").await?;

    let filter = req.worksite_filter();
    let request = http::Request::builder()
        .uri(format!("{MWS_URI}/worksite-search?filter={filter}"))
        .header("Content-Type", "application/json")
        .header("x-api-key", &api_key)
        .body(Empty::<Bytes>::new())
        .context("building request")?;
    let response = HttpRequest::fetch(provider, request)
        .await
        .map_err(|e| bad_gateway!("issue fetching worksites: {e}"))?;

    let bytes = response.into_body();
    let worksites: Vec<Worksite> =
        serde_json::from_slice(&bytes).context("deserializing worksites response")?;
    if worksites.is_empty() {
        Err(Error::NotFound {
            code: "not_found".to_string(),
            description: format!("no worksites found for code {}", req.worksite_code),
        })?;
    }

    let mut worksite = worksites[0].clone();

    // Optionally fetch related TMPs
    if req.include_tmps.unwrap_or(true) {
        let filter = req.tmp_filter();
        let request = http::Request::builder()
            .uri(format!("{MWS_URI}/tmp-search?filter={filter}"))
            .header("Content-Type", "application/json")
            .header("x-api-key", &api_key)
            .body(Empty::<Bytes>::new())
            .context("building request")?;
        let response = HttpRequest::fetch(provider, request)
            .await
            .map_err(|e| bad_gateway!("issue fetching TMPs: {e}"))?;

        let bytes = response.into_body();
        let tmps: Vec<Tmp> =
            serde_json::from_slice(&bytes).context("deserializing TMPs response")?;
        worksite.tmps = Some(tmps);
    }

    Ok(WorksiteResponse(worksite))
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct WorksiteRequest {
    pub worksite_code: String,
    #[serde(rename = "expanded")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub include_tmps: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub date_from: Option<NaiveDate>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub date_to: Option<NaiveDate>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct WorksiteResponse(pub Worksite);

impl IntoBody for WorksiteResponse {
    fn into_body(self) -> anyhow::Result<Vec<u8>> {
        serde_json::to_vec(&self).context("serializing reply")
    }
}

impl<P> Handler<P> for WorksiteRequest
where
    P: Config + HttpRequest,
{
    type Error = Error;
    type Input = Option<String>;   // <-- query string
    type Output = WorksiteResponse;

    fn from_input(input: Option<String>) -> Result<Self> {
        let request = serde_urlencoded::from_str(&input.unwrap_or_default())
            .context("deserializing request")?;
        Ok(request)
    }

    async fn handle(self, ctx: Context<'_, P>) -> Result<Reply<WorksiteResponse>> {
        Ok(handle(ctx.owner, self, ctx.provider).await?.into())
    }
}

impl WorksiteRequest {
    fn worksite_id(&self) -> &str {
        self.worksite_code.trim_start_matches(|c: char| c.is_alphabetic() || c == '-')
    }

    fn worksite_filter(&self) -> String {
        Filter::new()
            .condition("worksiteId", Cmp::Eq(json!(self.worksite_id())))
            .field("location", false)
            .to_encoded()
    }

    fn tmp_filter(&self) -> String {
        let filter = Filter::new()
            .condition("worksiteId", Cmp::Eq(json!(self.worksite_id())));

        let (Some(date_from), Some(date_to)) = (&self.date_from, &self.date_to) else {
            return filter.to_encoded();
        };

        let date_from = date_from.and_hms_opt(0, 0, 0).unwrap_or_default().and_utc();
        let date_to = date_to.and_hms_opt(23, 59, 59).unwrap_or_default().and_utc();
        filter
            .condition("layoutMaxDate", Cmp::Gte(json!(date_from)))
            .condition("layoutMinDate", Cmp::Lte(json!(date_to)))
            .to_encoded()
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Tmp {
    pub tmp_id: i64,
    pub tmp_code: Option<String>,
    pub worksite_id: Option<i32>,
    pub worksite_name: Option<String>,
    // ... additional fields
}
```

### Key Patterns

- `type Input = Option<String>` for query string handlers
- `serde_urlencoded::from_str` for parsing URL-encoded parameters
- Optional related data fetching controlled by a boolean parameter
- Filter builder methods on the request struct itself
- Date range filtering with `NaiveDate` -> UTC conversion
- `#[serde(rename = "expanded")]` for wire-name aliasing on request fields

## src/filter.rs

Utility module for building API query filters. Demonstrates a builder pattern for constructing URL-encoded JSON filters.

```rust
//! Utilities for building MyWorkSites API query filters.

use std::fmt::Display;

use percent_encoding::{AsciiSet, CONTROLS, utf8_percent_encode};
use serde::{Deserialize, Serialize, Serializer, ser};
use serde_json::{Map, Value, json};

const URL: &AsciiSet =
    &CONTROLS.add(b' ').add(b'"').add(b',').add(b':').add(b'{').add(b'}').add(b'[').add(b']');

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Filter {
    #[serde(rename = "where")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub where_: Option<Where>,

    #[serde(skip_serializing_if = "Option::is_none")]
    fields: Option<Value>,

    #[serde(skip_serializing_if = "Option::is_none")]
    include: Option<Vec<Dataset>>,
    // ... order, limit, offset
}

impl Filter {
    pub const fn new() -> Self {
        Self { where_: None, fields: None, include: None }
    }

    pub fn condition(mut self, property: impl Into<String>, compare: Cmp) -> Self {
        let property = property.into();
        match &mut self.where_ {
            None => {
                self.where_ = Some(Where::Only(Condition { property, compare }));
            }
            Some(Where::Only(existing)) => {
                self.where_ =
                    Some(Where::And(vec![existing.clone(), Condition { property, compare }]));
            }
            Some(Where::And(conds)) => {
                conds.push(Condition { property, compare });
            }
        }
        self
    }

    pub fn field(mut self, field: impl Into<String>, include: bool) -> Self {
        let mut map = match self.fields {
            Some(Value::Object(map)) => map,
            _ => Map::new(),
        };
        map.insert(field.into(), json!(include));
        self.fields = Some(json!(map));
        self
    }

    pub fn to_encoded(&self) -> String {
        utf8_percent_encode(&self.to_string(), URL).to_string()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Cmp {
    Eq(Value),
    Neq(Value),
    Gt(Value),
    Gte(Value),
    Lt(Value),
    Lte(Value),
    Inq(Value),
}
```

### Key Patterns

- Builder pattern with method chaining
- URL encoding for query parameters
- Custom serialization for filter conditions
- `serde_json::Value` for flexible comparison values
