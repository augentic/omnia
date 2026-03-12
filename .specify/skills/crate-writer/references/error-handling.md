# Error Handling Conventions

This is the **canonical reference** for error handling in Omnia business logic crates. It covers macro usage, stable codes, and boundary conversion patterns.

## What an Error "Is" in Omnia

Errors are treated as:

- **A classification** (e.g., `BadRequest`, `BadGateway`, `ServerError`)
- **A stable machine code** (`code: String`) for logs/metrics/clients
- **A human description** (`description: String`) for debugging

This is why we return SDK errors (`omnia_sdk::Error`) instead of ad-hoc `anyhow::Error` at the domain layer.

## `omnia_sdk::Error` Enum Definition

The SDK error enum has four variants, each with `code` and `description` fields:

```rust
#[derive(Error, Debug, Clone, Serialize, Deserialize)]
pub enum Error {
    /// Request payload is invalid or missing required fields. (HTTP 400)
    #[error("code: {code}, description: {description}")]
    BadRequest { code: String, description: String },

    /// Resource or data not found. (HTTP 404)
    #[error("code: {code}, description: {description}")]
    NotFound { code: String, description: String },

    /// A non-recoverable internal error occurred. (HTTP 500)
    #[error("code: {code}, description: {description}")]
    ServerError { code: String, description: String },

    /// An upstream dependency failed while fulfilling the request. (HTTP 502)
    #[error("code: {code}, description: {description}")]
    BadGateway { code: String, description: String },
}
```

**CRITICAL**: Do NOT use method-style constructors like `Error::bad_request("message")` or `Error::not_found("message")` -- these methods do NOT exist on the enum. Use either:

1. **Macros** (preferred): `bad_request!("message")`, `server_error!("message")`, `bad_gateway!("message")`
2. **Struct variant construction**: `Error::BadRequest { code: "code".to_string(), description: "message".to_string() }`
3. **Domain error conversion**: `impl From<DomainError> for Error` (see Pattern 2 below)

```rust
// CORRECT -- use macros
return Err(bad_request!("missing field"));

// CORRECT -- struct variant
return Err(Error::BadRequest { code: "no_update".to_string(), description: err.to_string() });

// WRONG -- method-style constructors DO NOT EXIST
return Err(Error::bad_request("missing field"));       // DOES NOT COMPILE
return Err(Error::not_found("customer not found"));    // DOES NOT COMPILE
```

## Standard Error Flow (Domain → Boundary → HTTP)

1. **Domain crate** returns `omnia_sdk::Result<Reply<T>>`
2. Domain code creates errors using `omnia_sdk::Error` + macros (`bad_request!`, `server_error!`, `bad_gateway!`)
3. Domain code adds context with `anyhow::Context`
4. **Boundary code** converts domain errors into HTTP responses via `omnia_sdk::api::HttpError`

## Error Macros

### `bad_request!` — Input Validation Failures (400)

```rust
use omnia_sdk::bad_request;

// Simple message (description only)
let err = bad_request!("missing vehicle identifier");

// With formatting
let err = bad_request!("invalid timestamp: {}", timestamp);

// With stable code + description (preferred for domain errors)
let err = bad_request!("customer_not_found", "Customer not found");
let err = bad_request!("empty_order", format!("Order {} has no items", order_id));

// In validation
let vehicle_id = message.vehicle_id()
    .ok_or_else(|| bad_request!("missing vehicle identifier"))?;
```

The two-argument form `bad_request!(code, description)` is preferred when you want a stable machine-readable error code for logs, metrics, and clients. The single-argument form uses the message as both code and description.

**Use for:**

- Missing required fields
- Invalid input format
- Parsing failures
- Validation errors

### `server_error!` — Internal Failures (500)

```rust
use omnia_sdk::server_error;

// Internal invariant violations
let err = server_error!("unexpected state: {}", state);

// Serialization failures
let payload = serde_json::to_vec(&event)
    .map_err(|err| server_error!("failed to serialize event: {err}"))?;
```

**Use for:**

- Internal invariant violations
- Unexpected states
- Serialization errors (internal)

### `bad_gateway!` — Upstream Failures (502)

```rust
use omnia_sdk::bad_gateway;

// Upstream API failure
let response = HttpRequest::fetch(provider, request)
    .await
    .map_err(|err| bad_gateway!("upstream request failed: {err}"))?;
```

**Use for:**

- Upstream API failures
- External service errors
- Dependency failures you want to surface as 502

## Pattern 1: Input Validation via `bad_request!`

The most common pattern: validate early, return 400-class error.

```rust
use omnia_sdk::{bad_request, Result};

pub fn validate_message(message: &InboundMessage) -> Result<()> {
    // Check required fields
    if message.vehicle_id.is_empty() {
        return Err(bad_request!("vehicle_id is required"));
    }

    // Parse and validate timestamp
    let ts = DateTime::parse_from_rfc3339(&message.timestamp)
        .map_err(|e| bad_request!("invalid timestamp: {e}"))?;

    // Validate ranges
    if message.latitude < -90.0 || message.latitude > 90.0 {
        return Err(bad_request!("latitude out of range: {}", message.latitude));
    }

    Ok(())
}
```

## Pattern 2: Domain Error Enums with Stable Codes

When you need named domain failures with stable codes:

```rust
use thiserror::Error;
use omnia_sdk::Error as SdkError;

#[derive(Error, Debug)]
pub enum DomainError {
    #[error("invalid timestamp: {0}")]
    BadTimestamp(String),

    #[error("vehicle not found: {0}")]
    VehicleNotFound(String),

    #[error("enrichment failed: {0}")]
    EnrichmentFailed(String),
}

impl DomainError {
    /// Returns a stable machine-readable error code.
    fn code(&self) -> &'static str {
        match self {
            Self::BadTimestamp(_) => "bad_timestamp",
            Self::VehicleNotFound(_) => "vehicle_not_found",
            Self::EnrichmentFailed(_) => "enrichment_failed",
        }
    }
}

impl From<DomainError> for SdkError {
    fn from(err: DomainError) -> Self {
        match &err {
            DomainError::BadTimestamp(_) => SdkError::BadRequest {
                code: err.code().to_string(),
                description: err.to_string(),
            },
            DomainError::VehicleNotFound(_) => SdkError::NotFound {
                code: err.code().to_string(),
                description: err.to_string(),
            },
            DomainError::EnrichmentFailed(_) => SdkError::BadGateway {
                code: err.code().to_string(),
                description: err.to_string(),
            },
        }
    }
}
```

**Key Points:**

- `code()` returns a **stable snake_case identifier** (avoid embedding variable values)
- The `Display` message (via `thiserror`) becomes the `description`
- The conversion chooses the HTTP class

## Pattern 3: Adding Context with `anyhow::Context`

Use `anyhow::Context` for serialization/decoding/formatting failures:

```rust
use anyhow::Context as _;
use omnia_sdk::Result;

pub fn parse_message(input: &[u8]) -> Result<MyMessage> {
    serde_json::from_slice(input)
        .context("deserializing MyMessage")
        .map_err(Into::into)
}

pub async fn fetch_and_parse<P: HttpRequest>(provider: &P, url: &str) -> Result<Data> {
    let response = HttpRequest::fetch(provider, request)
        .await
        .context("fetching data")?;

    let data: Data = serde_json::from_slice(response.body())
        .context("parsing response")
        .map_err(Into::into)?;

    Ok(data)
}
```

**Why:** You keep a readable error chain without losing the SDK's classification model.

## Pattern 4: Handler Error Pattern

Handlers use `from_input` for parsing and `handle` for business logic:

```rust
use anyhow::Context as _;
use omnia_sdk::{Context, Error, Handler, Reply, Result};

impl<P: Config> Handler<P> for MyRequest {
    type Error = Error;
    type Input = Vec<u8>;
    type Output = MyResponse;

    fn from_input(input: Self::Input) -> Result<Self> {
        serde_json::from_slice(&input)
            .context("deserializing MyRequest")
            .map_err(Into::into)
    }

    async fn handle(self, ctx: Context<'_, P>) -> Result<Reply<MyResponse>> {
        // Validation
        self.validate()?;

        // Business logic
        let result = process(ctx.provider, &self).await?;

        Ok(result.into())
    }
}
```

## Pattern 5: Boundary Conversion

At the HTTP boundary, preserve classification with `.map_err(Into::into)`:

```rust
// In boundary code (NOT generated by this skill)
use omnia_sdk::api::HttpResult;

pub async fn route(body: Bytes, provider: &Provider) -> HttpResult<Reply<MyResponse>> {
    MyRequest::handler(body.to_vec())?
        .owner("at")
        .provider(provider)
        .await
        .map_err(Into::into)
}
```

**Why:** This ensures the SDK error becomes the appropriate HTTP status (400/404/500/502) while preserving the stable `code`.

## Error Model Fidelity (Code-Analysis Artifacts)

When migrating from existing source code via code-analysis artifacts, preserve the source's error model:

- **Match error granularity**: If the source defines 3 error variants (`BadTime`, `NoUpdate`, `InvalidXml`), generate exactly 3 -- not 7 fine-grained variants. Over-splitting errors changes the API contract for downstream consumers that match on error codes.
- **Match error codes**: If the source uses `"bad_time"` as an error code, use `"bad_time"` -- do not rename to `"outdated"` or `"wrong_time"`.
- **Use string payloads when the source does**: If the source uses `BadTime(String)` to carry context, generate `BadTime(String)` -- not `Outdated(i64)` + `WrongTime(i64)`.

```rust
// CORRECT -- matches source error model (3 variants, string payloads)
#[derive(Error, Debug)]
pub enum R9kError {
    #[error("{0}")]
    BadTime(String),

    #[error("{0}")]
    NoUpdate(String),

    #[error("{0}")]
    InvalidXml(String),
}

// WRONG -- invented 7 variants not in source, different error codes
#[derive(Error, Debug)]
pub enum R9kError {
    NoUpdate,                    // code: "no_update"
    NoActualUpdate,              // code: "no_actual_update" (invented)
    Outdated(i64),               // code: "outdated" (was "bad_time")
    WrongTime(i64),              // code: "wrong_time" (was "bad_time")
    InvalidDate(String),         // code: "invalid_date" (invented)
    IrrelevantStation(u32),      // code: "irrelevant_station" (invented)
    NotMovementEvent,            // code: "not_movement_event" (invented)
}
```

## Error Code Conventions

### Stable Codes (DO)

```rust
// ✅ Good: Stable, predictable codes
"bad_timestamp"
"vehicle_not_found"
"enrichment_failed"
"parse_error"
"missing_field"
```

### Avoid Variable Codes (DON'T)

```rust
// ❌ Bad: Variable data in codes
format!("bad_timestamp_{}", field_name)  // Don't embed variable
format!("error_{}", error_code)           // Don't use dynamic codes
```

## Required Imports

```rust
// Error handling imports
use omnia_sdk::{Error, Result, bad_request, server_error, bad_gateway};
use anyhow::Context as _;
use thiserror::Error;
```

## Summary Table

| Situation                | Macro/Pattern                  | HTTP Status |
| ------------------------ | ------------------------------ | ----------- |
| Missing field            | `bad_request!`                 | 400         |
| Invalid format           | `bad_request!`                 | 400         |
| Parse error              | `bad_request!` or `.context()` | 400         |
| Not found                | `DomainError::NotFound`        | 404         |
| Internal error           | `server_error!`                | 500         |
| Upstream failure         | `bad_gateway!`                 | 502         |
| Serialization (internal) | `server_error!`                | 500         |
| Deserialization (input)  | `bad_request!`                 | 400         |

## Troubleshooting

### Create Mode

| Issue | Cause | Resolution |
| --- | --- | --- |
| TypeScript source doesn't parse | Invalid TypeScript or missing dependencies | Run `tsc --noEmit` to verify source compiles first |
| Too many [unknown] tags | Dynamic typing, metaprogramming, or unclear logic | Review source for type annotations; add comments for clarity |
| Artifacts missing business logic | Functions not exported or in inaccessible modules | Check module imports; ensure key functions are exported |
| Config keys not captured | Environment variables accessed indirectly | Search for `process.env` patterns across all source files |

### Update Mode

| Issue | Cause | Resolution |
| --- | --- | --- |
| Baseline `cargo test` fails to compile | Existing crate has compilation errors | Record errors; do not introduce additional failures; fix if the update touches affected code |
| Change classification ambiguous | Artifact difference could be modifying or structural | Prefer the simpler classification (modifying over structural); see [change-classification.md](change-classification.md) |
| Structural change breaks compilation | Rename or restructure missed a reference | Re-scan crate after structural changes (Hard Rule 16); fix remaining references |
| Regression detected | Previously-passing test now fails | Compare handler implementation against updated artifacts; repair using the strategies above |
| Artifacts remove behavior that other crates depend on | Cross-crate dependency on removed handler/type | Document in CHANGELOG.md; mark removal with `// BREAKING:` comment; warn in Migration.md |
| MockProvider missing new trait | Handler gained a new provider bound | Add trait impl to `tests/provider.rs` with appropriate test fixtures |
| Guest wiring conflict | Route path or topic already exists for a different handler | Update the existing route rather than adding a duplicate; document the replacement in CHANGELOG.md |
| Update too complex for automation | Complete handler rewrite or fundamental architecture change | Abort and recommend re-running the greenfield pipeline (create mode); document rationale |

## References

- See [sdk-api.md](sdk-api.md) for Handler, Context, Reply types
- See [guardrails.md](guardrails.md) for error model guardrails
