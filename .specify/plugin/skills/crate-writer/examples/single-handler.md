# Single Handler Example: r9k-adapter

A messaging adapter crate with a single handler that receives XML messages, validates them, transforms them into domain events, and publishes to a topic. From the `train` project.

## Crate Structure

```
crates/r9k-adapter/
├── Cargo.toml
├── src/
│   ├── lib.rs          # Module declarations, error type, re-exports
│   ├── handler.rs      # Handler<P> impl + standalone handle fn
│   ├── r9k.rs          # Input types (XML deserialization, validation)
│   ├── smartrak.rs     # Output types (event serialization)
│   └── stops.rs        # Domain helper (stop info lookup)
├── tests/
│   ├── provider.rs     # MockProvider
│   └── static.rs       # Integration tests
└── data/
    └── static/         # JSON test fixtures
```

## src/lib.rs

Module declarations, domain error enum, and re-exports. Small crates define errors here; larger crates use a separate `error.rs`.

```rust
//! # R9K Transformer
//!
//! Transforms R9K messages into SmarTrak events.

mod handler;
mod r9k;
mod smartrak;
mod stops;

use omnia_sdk::Error;
use thiserror::Error;

pub use self::handler::*;
pub use self::r9k::*;
pub use self::smartrak::*;
pub use self::stops::StopInfo;

#[derive(Error, Debug)]
pub enum R9kError {
    #[error("{0}")]
    BadTime(String),

    #[error("{0}")]
    NoUpdate(String),

    #[error("{0}")]
    InvalidXml(String),
}

impl R9kError {
    fn code(&self) -> String {
        match self {
            Self::BadTime(_) => "bad_time".to_string(),
            Self::NoUpdate(_) => "no_update".to_string(),
            Self::InvalidXml(_) => "invalid_message".to_string(),
        }
    }
}

impl From<R9kError> for Error {
    fn from(err: R9kError) -> Self {
        Self::BadRequest { code: err.code(), description: err.to_string() }
    }
}

impl From<quick_xml::DeError> for R9kError {
    fn from(err: quick_xml::DeError) -> Self {
        Self::InvalidXml(err.to_string())
    }
}
```

### Key Patterns

- Domain errors derive `thiserror::Error`
- Each variant has a stable `code()` method for machine-readable error codes
- `From<DomainError> for omnia_sdk::Error` maps all domain errors to an appropriate SDK variant
- Additional `From` impls for library errors (e.g., `quick_xml::DeError`)

## src/handler.rs

The Handler implementation and standalone handle function.

```rust
//! R9K Position Adapter

use anyhow::Context as _;
use bytes::Bytes;
use chrono::Utc;
use http::header::AUTHORIZATION;
use http_body_util::Empty;
use omnia_sdk::api::{Context, Handler, Reply};
use omnia_sdk::{Config, Error, HttpRequest, Identity, Message, Publish, Result};
use serde::Deserialize;

use crate::r9k::TrainUpdate;
use crate::smartrak::{EventType, MessageData, RemoteData, SmarTrakEvent};
use crate::stops;

const SMARTRAK_TOPIC: &str = "realtime-r9k-to-smartrak.v1";

/// R9K train update message as deserialized from XML.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct R9kMessage {
    #[serde(rename(deserialize = "ActualizarDatosTren"))]
    pub train_update: TrainUpdate,
}

// Standalone handle function -- business logic lives here
async fn handle<P>(owner: &str, request: R9kMessage, provider: &P) -> Result<Reply<()>>
where
    P: Config + HttpRequest + Identity + Publish,
{
    let update = request.train_update;
    update.validate()?;

    let events = update.into_events(owner, provider).await?;

    let env = Config::get(provider, "ENV").await.unwrap_or_else(|_| "dev".to_string());
    let topic = format!("{env}-{SMARTRAK_TOPIC}");

    for _ in 0..2 {
        #[cfg(not(debug_assertions))]
        std::thread::sleep(std::time::Duration::from_secs(5));

        for event in &events {
            tracing::info!(monotonic_counter.smartrak_events_published = 1);

            let payload = serde_json::to_vec(&event).context("serializing event")?;
            let external_id = &event.remote_data.external_id;

            let mut message = Message::new(&payload);
            message.headers.insert("key".to_string(), external_id.clone());

            Publish::send(provider, &topic, &message).await?;
        }
    }

    Ok(Reply::ok(()))
}

// Handler trait -- delegates to standalone function
impl<P> Handler<P> for R9kMessage
where
    P: Config + HttpRequest + Identity + Publish,
{
    type Error = Error;
    type Input = Vec<u8>;
    type Output = ();

    fn from_input(input: Vec<u8>) -> Result<Self> {
        quick_xml::de::from_reader(input.as_ref())
            .context("deserializing R9kMessage")
            .map_err(Into::into)
    }

    async fn handle(self, ctx: Context<'_, P>) -> Result<Reply<()>> {
        handle(ctx.owner, self, ctx.provider).await
    }
}
```

### Key Patterns

- `from_input` uses `.context("...").map_err(Into::into)` -- never wraps in domain errors
- `handle()` delegates to a standalone `async fn handle<P>(...)`
- Provider bounds list exactly the traits needed: `Config + HttpRequest + Identity + Publish`
- Topic naming uses `{env}-{CONSTANT}` pattern
- Repeated publication: `for _ in 0..N { sleep; publish; }` with no payload mutation
- `std::thread::sleep` guarded by `#[cfg(not(debug_assertions))]`

## src/r9k.rs

Input types with XML field mappings, validation logic, and typed enums.

```rust
//! R9K data types

use std::fmt::{Display, Formatter};

use chrono::{NaiveDate, Utc};
use chrono_tz::Pacific;
use omnia_sdk::Result;
use serde::Deserialize;
use serde_repr::Deserialize_repr;

use crate::R9kError;

const MAX_DELAY_SECS: i64 = 60;
const MIN_DELAY_SECS: i64 = -30;

/// R9K train update as received from KiwiRail (Spanish XML field names).
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct TrainUpdate {
    #[serde(rename(deserialize = "trenPar"))]
    pub even_train_id: Option<String>,

    #[serde(rename(deserialize = "trenImpar"))]
    pub odd_train_id: Option<String>,

    #[serde(rename(deserialize = "fechaCreacion"))]
    #[serde(deserialize_with = "r9k_date")]
    pub created_date: NaiveDate,

    #[serde(rename(deserialize = "operadorComercial"))]
    pub train_type: TrainType,

    #[serde(rename(deserialize = "pasoTren"), default)]
    pub changes: Vec<Change>,
}

// Custom date deserializer for dd/mm/yyyy format
fn r9k_date<'de, D>(deserializer: D) -> anyhow::Result<NaiveDate, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let s: String = Deserialize::deserialize(deserializer)?;
    NaiveDate::parse_from_str(&s, "%d/%m/%Y").map_err(serde::de::Error::custom)
}

impl TrainUpdate {
    #[must_use]
    pub fn train_id(&self) -> String {
        self.even_train_id.clone()
            .unwrap_or_else(|| self.odd_train_id.clone().unwrap_or_default())
    }

    /// Temporal validation -- uses Utc::now(), so must be in handle() not from_input().
    pub fn validate(&self) -> Result<()> {
        if self.changes.is_empty() {
            return Err(R9kError::NoUpdate("contains no updates".to_string()).into());
        }

        let change = &self.changes[0];
        let since_midnight_secs = if change.has_departed {
            change.actual_departure_time
        } else if change.has_arrived {
            change.actual_arrival_time
        } else {
            return Err(R9kError::NoUpdate("arrival/departure time <= 0".to_string()).into());
        };

        if since_midnight_secs <= 0 {
            return Err(R9kError::NoUpdate("arrival/departure time <= 0".to_string()).into());
        }

        // Rebuild event timestamp from creation date + seconds from midnight
        let naive_dt = self.created_date.and_hms_opt(0, 0, 0).unwrap_or_default();
        let Some(midnight_dt) = naive_dt.and_local_timezone(Pacific::Auckland).earliest() else {
            return Err(R9kError::BadTime(format!("invalid local time: {naive_dt}")).into());
        };
        let event_ts = midnight_dt.timestamp() + i64::from(since_midnight_secs);

        let now_ts = Utc::now().with_timezone(&Pacific::Auckland).timestamp();
        let delay_secs = now_ts - event_ts;

        tracing::info!(gauge.r9k_delay = delay_secs);

        if delay_secs > MAX_DELAY_SECS {
            return Err(R9kError::BadTime(format!("outdated by {delay_secs} seconds")).into());
        }
        if delay_secs < MIN_DELAY_SECS {
            return Err(
                R9kError::BadTime(format!("too early by {} seconds", delay_secs.abs())).into()
            );
        }

        Ok(())
    }
}

/// Integer-backed enum using serde_repr.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize_repr)]
#[repr(u8)]
pub enum ChangeType {
    ExitedFirstStation = 1,
    ReachedFinalDestination = 2,
    ArrivedAtStation = 3,
    ExitedStation = 4,
    PassedStationWithoutStopping = 5,
    DetainedInPark = 6,
    DetainedAtStation = 7,
    StationNoLongerPartOfTheRun = 8,
    PlatformChange = 9,
    ExitLineChange = 10,
    ScheduleChange = 11,
}

impl ChangeType {
    #[must_use]
    pub const fn is_relevant(&self) -> bool {
        matches!(
            self,
            Self::ReachedFinalDestination
                | Self::ArrivedAtStation
                | Self::ExitedFirstStation
                | Self::ExitedStation
                | Self::PassedStationWithoutStopping
                | Self::ScheduleChange
        )
    }

    #[must_use]
    pub const fn is_arrival(&self) -> bool {
        matches!(self, Self::ArrivedAtStation | Self::ReachedFinalDestination)
    }
}

/// String-backed enum.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum TrainType {
    #[default]
    Metro,
    Exmetro,
    Freight,
}

/// Integer-backed enum with serde_repr.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize_repr)]
#[repr(i8)]
pub enum Direction {
    Right = 0,
    Left = 1,
    Unspecified = -1,
}
```

### Key Patterns

- Input types: `Deserialize` only (not `Serialize`), `#[serde(default)]`, `#[serde(rename(deserialize = "..."))]`
- `Option<String>` for fields that may be absent
- Integer enums use `serde_repr::Deserialize_repr` with `#[repr(u8)]` or `#[repr(i8)]`
- String enums use `#[serde(rename_all = "...")]`
- Custom date deserializer for non-standard formats
- Validation using `Utc::now()` is in a `validate()` method called from `handle()`, not `from_input()`
- DST-safe timezone: `.earliest()` not `.single()`
- Constants for validation thresholds: `MAX_DELAY_SECS`, `MIN_DELAY_SECS`

## src/smartrak.rs

Output types with camelCase serialization and optional field handling.

```rust
//! SmarTrak event types.

use chrono::{DateTime, SecondsFormat, Utc};
use serde::{Deserialize, Serialize, Serializer};

use crate::stops::StopInfo;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SmarTrakEvent {
    #[serde(serialize_with = "with_nanos")]
    pub received_at: DateTime<Utc>,
    pub event_type: EventType,
    pub event_data: EventData,
    pub message_data: MessageData,
    pub remote_data: RemoteData,
    pub location_data: LocationData,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub company_id: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub serial_data: Option<SerialData>,
}

fn with_nanos<S>(dt: &DateTime<Utc>, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    let trunc = dt.to_rfc3339_opts(SecondsFormat::Millis, true);
    serializer.serialize_str(&trunc)
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub enum EventType {
    #[default]
    Location,
    SerialData,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteData {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub remote_id: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub remote_name: Option<String>,
    pub external_id: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LocationData {
    pub latitude: f64,
    pub longitude: f64,
    pub speed: i64,
    pub gps_accuracy: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub heading: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kilometric_point: Option<f64>,
}

// From conversion for domain type -> output type
impl From<StopInfo> for LocationData {
    fn from(stop: StopInfo) -> Self {
        Self { latitude: stop.stop_lat, longitude: stop.stop_lon, ..Self::default() }
    }
}
```

### Key Patterns

- Output types: `Serialize + Deserialize`, `#[serde(rename_all = "camelCase")]`
- `Default` on all output types for struct update syntax
- `#[serde(skip_serializing_if = "Option::is_none")]` on optional fields
- Custom serializer for timestamp formatting
- `From` trait for converting domain types to output types
- Struct update syntax: `Self { latitude: ..., longitude: ..., ..Self::default() }`

## src/stops.rs

Domain helper that performs an external API lookup.

```rust
use std::collections::HashMap;
use std::sync::LazyLock;

use anyhow::{Context, Result, anyhow};
use bytes::Bytes;
use http_body_util::Empty;
use omnia_sdk::{Config, HttpRequest, Identity, Publish};
use serde::{Deserialize, Serialize};

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct StopInfo {
    pub stop_code: String,
    pub stop_lat: f64,
    pub stop_lon: f64,
}

pub async fn stop_info<P>(
    _owner: &str, provider: &P, station: u32, is_arrival: bool,
) -> Result<Option<StopInfo>>
where
    P: Config + HttpRequest + Identity + Publish,
{
    if !ACTIVE_STATIONS.contains(&station) {
        return Ok(None);
    }

    let Some(stop_code) = STATION_STOP.get(&station) else {
        return Ok(None);
    };

    let url = Config::get(provider, "CC_STATIC_URL").await.context("getting `CC_STATIC_URL`")?;
    let request = http::Request::builder()
        .uri(format!("{url}/gtfs/stops?fields=stop_code,stop_lon,stop_lat"))
        .body(Empty::<Bytes>::new())
        .context("building request")?;
    let response = HttpRequest::fetch(provider, request).await.context("fetching stops")?;

    let bytes = response.into_body();
    let stops: Vec<StopInfo> = serde_json::from_slice(&bytes).context("deserializing stops")?;

    let Some(mut stop_info) = stops.into_iter().find(|s| s.stop_code == *stop_code) else {
        return Err(anyhow!("stop info not found for stop code {stop_code}"));
    };

    if !is_arrival {
        stop_info = DEPARTURES.get(&stop_info.stop_code).cloned().unwrap_or(stop_info);
    }

    Ok(Some(stop_info))
}

const ACTIVE_STATIONS: &[u32] = &[0, 19, 40];

static STATION_STOP: LazyLock<HashMap<u32, &str>> =
    LazyLock::new(|| HashMap::from([(0, "133"), (19, "9218"), (40, "134")]));

static DEPARTURES: LazyLock<HashMap<String, StopInfo>> = LazyLock::new(|| {
    HashMap::from([
        ("133".to_string(), StopInfo { stop_code: "133".to_string(), stop_lat: -36.84448, stop_lon: 174.76915 }),
        ("134".to_string(), StopInfo { stop_code: "134".to_string(), stop_lat: -37.20299, stop_lon: 174.90990 }),
    ])
});
```

### Key Patterns

- `LazyLock` for immutable compile-time lookup tables
- `const` arrays for small static data
- Generic over provider `P` with required trait bounds
- Returns `Option` for "not found" cases (not an error)
- Uses `anyhow::Context` for error chaining on each fallible step
