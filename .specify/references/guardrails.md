# Guardrails

Hard constraints for generated crates. All code must target `wasm32-wasip2`.

## Forbidden Crates

These crates are **never** allowed in generated code:

| Crate                | Reason                          | Alternative                                     |
| -------------------- | ------------------------------- | ----------------------------------------------- |
| `reqwest`            | Brings full HTTP client stack   | `HttpRequest::fetch` via provider               |
| `tokio` (as runtime) | Not WASM-compatible             | Only in `[dev-dependencies]` for tests          |
| `redis`              | Direct connection not available | `StateStore` via provider                       |
| `sqlx`, `diesel`     | Direct DB connection            | `TableStore` via provider                       |
| `hyper`              | Server-side HTTP                | `omnia-wasi-http` + axum                        |
| `dotenv`, `dotenvy`  | File system access              | `Config::get` via provider                      |
| `rand`               | RNG not available in WASM       | Accept IDs as input or derive deterministically |
| `uuid`               | Depends on `rand`               | Accept IDs as input                             |
| `std::process`       | No process spawning in WASM     | N/A                                             |
| `lazy_static`        | Use `std::sync::LazyLock`       | `LazyLock` for immutable lookup tables only     |

## Forbidden std APIs

These standard library APIs are not available in WASM guests:

| API                  | Reason                | Alternative                       |
| -------------------- | --------------------- | --------------------------------- |
| `std::env::var`      | No environment access | `Config::get` via provider        |
| `std::fs::*`         | No filesystem access  | `StateStore` or `HttpRequest`     |
| `std::net::*`        | No direct networking  | `HttpRequest::fetch` via provider |
| `std::process::*`    | No process management | N/A                               |
| `std::thread::spawn` | Single-threaded WASM  | Async patterns                    |

### Exceptions

- `std::thread::sleep` -- allowed **only** when guarded by `#[cfg(not(debug_assertions))]` for repeated publication timing patterns
- `std::sync::LazyLock` -- allowed for immutable, compile-time-known lookup tables
- `std::collections::HashMap` -- fully allowed
- `std::fmt`, `std::io` (in-memory), `std::str` -- fully allowed

## Statelessness

WASM components must be fully stateless. All state flows through function parameters or provider trait calls.

**Forbidden**:

```rust
// NO: mutable global state
static mut COUNTER: u32 = 0;
static STATE: OnceCell<AppState> = OnceCell::new();

// NO: caching in static variables
static CACHE: Mutex<HashMap<String, Vec<u8>>> = Mutex::new(HashMap::new());
```

**Allowed**:

```rust
// YES: immutable compile-time lookup tables
static STATION_STOP: LazyLock<HashMap<u32, &str>> =
    LazyLock::new(|| HashMap::from([(0, "133"), (19, "9218"), (40, "134")]));

// YES: compile-time constants
const ACTIVE_STATIONS: &[u32] = &[0, 19, 40];
const MAX_DELAY_SECS: i64 = 60;
```

## Error Handling

- No `unwrap()` or `expect()` in production code (allowed in tests)
- Use `anyhow::Context` for error chaining
- All errors must ultimately convert to `omnia_sdk::Error`
- Domain errors use `thiserror` + `From<DomainError> for omnia_sdk::Error`
- Use `bad_gateway!` for upstream API failures, `bad_request!` for input validation

## Code Quality

- No `println!`, `dbg!`, or `eprintln!` -- use `tracing::debug!` / `tracing::info!`
- No `unsafe` blocks
- Functions should be < 50 lines
- All public types and functions have doc comments
- Use `#[must_use]` on builder methods and pure functions

## Metrics and Logging

- Diagnostic messages: `tracing::debug!`
- Operational metrics: `tracing::info!` with OpenTelemetry-compatible names

```rust
tracing::info!(monotonic_counter.events_published = 1);
tracing::info!(gauge.processing_delay = delay_secs);
tracing::debug!("fetched {} layouts", layouts.len());
```

## Serde Rules

### Input-Only Types (XML/JSON that are deserialized but not re-serialized with wire names)

```rust
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct InboundMessage {
    #[serde(rename(deserialize = "sourceField"))]
    pub source_field: Option<String>,
}
```

- `#[serde(rename(deserialize = "..."))]` -- deserialize-only rename
- `#[serde(default)]` -- tolerate missing fields
- `Option<T>` for fields that may be absent
- Derive `Default` + `Deserialize` (not `Serialize` unless needed for caching)

### Output Types (JSON responses, published events)

```rust
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OutputEvent {
    pub received_at: DateTime<Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub optional_field: Option<String>,
}
```

- `#[serde(rename_all = "camelCase")]` at struct level
- `#[serde(skip_serializing_if = "Option::is_none")]` on optional fields
- Derive `Clone, Debug, Default, Serialize, Deserialize`

### Integer-Backed Enums

```rust
use serde_repr::Deserialize_repr;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize_repr)]
#[repr(u8)]
pub enum ChangeType {
    Created = 1,
    Updated = 2,
    Deleted = 3,
}
```

Never deserialize integer enums as raw `u32` with manual conversion.

### Custom Date Parsing

```rust
#[serde(deserialize_with = "custom_date")]
pub created_date: NaiveDate,

fn custom_date<'de, D>(deserializer: D) -> Result<NaiveDate, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let s: String = Deserialize::deserialize(deserializer)?;
    NaiveDate::parse_from_str(&s, "%d/%m/%Y").map_err(serde::de::Error::custom)
}
```

### Custom Timestamp Serialization

```rust
#[serde(serialize_with = "with_millis")]
pub received_at: DateTime<Utc>,

fn with_millis<S>(dt: &DateTime<Utc>, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    serializer.serialize_str(&dt.to_rfc3339_opts(SecondsFormat::Millis, true))
}
```

## DST-Safe Timezone Conversion

Use `.earliest()` not `.single()` when converting local times:

```rust
let naive_dt = date.and_hms_opt(0, 0, 0).unwrap_or_default();
let Some(midnight) = naive_dt.and_local_timezone(Pacific::Auckland).earliest() else {
    return Err(MyError::BadTime(format!("invalid local time: {naive_dt}")).into());
};
```

`.single()` returns `None` during DST transitions; `.earliest()` picks the first valid interpretation.

## Timestamp Semantics

- `received_at` -- always `Utc::now()` (when the handler processed the message)
- `timestamp` (in message_data) -- `Utc::now()` unless artifacts explicitly say otherwise
- Source creation dates map to named fields like `created_at`, not `received_at`
