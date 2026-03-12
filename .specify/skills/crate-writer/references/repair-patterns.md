# Repair Patterns

Quick-reference for canonical Omnia SDK patterns. Consult when fixing test failures or compilation errors in generated crates.

## Handler<P> Structure

Request struct implements `Handler<P>`, delegating to a standalone async function:

```rust
async fn handle<P>(_owner: &str, request: MyRequest, provider: &P) -> Result<Reply<MyResponse>>
where
    P: Config + HttpRequest,
{
    // business logic here
    Ok(Reply::ok(response))
}

impl<P> Handler<P> for MyRequest
where
    P: Config + HttpRequest,
{
    type Error = Error;
    type Input = Vec<u8>;
    type Output = MyResponse;

    fn from_input(input: Vec<u8>) -> Result<Self> {
        serde_json::from_slice(&input)
            .context("deserializing MyRequest")
            .map_err(Into::into)
    }

    async fn handle(self, ctx: Context<'_, P>) -> Result<Reply<MyResponse>> {
        handle(ctx.owner, self, ctx.provider).await
    }
}
```

**Rules**:

- Never use custom handler structs with `new()` / `process_message()` -- only `Handler<P>` on request types
- Never use `type Input = MyRequest` -- bypasses deserialization
- `type Error` is always `Error` (from `omnia_sdk`)

## Input Type Decision Tree

| Scenario                             | `type Input`       | `from_input` pattern                                     |
| ------------------------------------ | ------------------ | -------------------------------------------------------- |
| Message/POST body                    | `Vec<u8>`          | `serde_json::from_slice` or `quick_xml::de::from_reader` |
| Single path param (`GET /item/{id}`) | `String`           | `Ok(Self { id: input })`                                 |
| Tuple path params (`GET /a/{x}/{y}`) | `(String, String)` | `Ok(Self { x: input.0, y: input.1 })`                    |
| Query string (`GET /search?q=...`)   | `Option<String>`   | `serde_urlencoded::from_str(&input.unwrap_or_default())` |
| Scheduled/cron (no payload)          | `()`               | `Ok(Self)`                                               |

## Response Types

HTTP response types implement `IntoBody`:

```rust
impl IntoBody for MyResponse {
    fn into_body(self) -> anyhow::Result<Vec<u8>> {
        serde_json::to_vec(&self).context("serializing reply")
    }
}
```

Messaging handlers use `type Output = ()` and do not need `IntoBody`.

## Validation Placement

```text
Does the validation use Utc::now(), SystemTime, or runtime state?
├─ YES → belongs in handle() or validate() method called from handle()
└─ NO → can this check be done immediately after parsing?
   ├─ YES → belongs in from_input()
   └─ NO → belongs in handle()
```

- **Structural** (field presence, format, range) → `from_input()`
- **Temporal/contextual** (timestamp freshness, idempotency, external lookups) → `handle()`
- Never use `Utc::now()` in `from_input()` -- breaks `shift_time` in replay tests

## Error Handling

Domain errors use `thiserror` and convert to `omnia_sdk::Error`:

```rust
#[derive(Error, Debug)]
pub enum MyError {
    #[error("{0}")]
    InvalidInput(String),
}

impl MyError {
    fn code(&self) -> String {
        match self {
            Self::InvalidInput(_) => "invalid_input".to_string(),
        }
    }
}

impl From<MyError> for Error {
    fn from(err: MyError) -> Self {
        Self::BadRequest { code: err.code(), description: err.to_string() }
    }
}
```

**Error macros** for one-off errors: `bad_request!("msg")`, `server_error!("msg")`, `bad_gateway!("msg")`.

**CRITICAL**: Method-style constructors like `Error::bad_request("msg")` DO NOT EXIST. Use macros or struct variants.

Replace `unwrap()` / `expect()` with `?` and proper error context:

```rust
// WRONG
let customer = customers.get(&id).unwrap();

// CORRECT
let customer = customers.get(&id)
    .ok_or_else(|| bad_request!("customer not found: {id}"))?;
```

## Serde Conventions

- **Input-only types**: `#[serde(rename(deserialize = "fieldName"))]` + `#[serde(default)]` on struct
- **Output types**: `#[serde(rename_all = "camelCase")]` at struct level
- **Round-trip types**: `#[serde(rename = "...")]` or `#[serde(rename_all = "...")]`

Wrong: `#[serde(rename = "sourceField")]` on input-only types (causes foreign field names in serialized output).

## Provider Bound Composition

- Include **only** the traits the function actually calls
- Handler bounds = union of all traits needed by functions it calls
- Never construct host-side types (`Client::new()`, `RedisClient::connect()`)
- Never wrap provider traits in custom abstractions
- Use `Config::get(provider, "KEY")` not `std::env::var("KEY")`

## Timestamp Semantics

- `received_at` / `receivedAt` → always `Utc::now()` (processing time)
- Source creation dates → named fields like `created_at`, `source_timestamp`
- DST-safe conversion: use `.earliest()` not `.single()` for local timezone conversion

## Clippy Warnings

Apply the suggested fix from the clippy output. Common patterns:

- `unnecessary_unwrap` → replace with `?` or `if let`
- `manual_let_else` → use `let ... else { return }`
- `collapsible_if` → combine nested if-blocks
- `doc_markdown` → backtick-wrap code identifiers, `#[allow(clippy::doc_markdown)]` for proper nouns
- `match_same_arms` → merge identical match arms with `|`
- `unnecessary_map_or` → use `is_some_and` or `is_ok_and`
