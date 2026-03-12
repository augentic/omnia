# Omnia SDK API Reference

Extracted from `omnia-sdk` crate source. This is the definitive API surface for generated crates.

## Handler Trait

The core trait implemented by all request types. Defined in `omnia_sdk::api`.

```rust
pub trait Handler<P: Provider>: Sized {
    /// Raw input type (Vec<u8>, String, (String, String), Option<String>, or ()).
    type Input;

    /// Response body type. Must implement Body (Debug + Send + Sync).
    type Output: Body;

    /// Always omnia_sdk::Error.
    type Error: Error + Send + Sync;

    /// Parse raw input into a handler instance.
    fn from_input(input: Self::Input) -> Result<Self, Self::Error>;

    /// Convenience: parse input and wrap in a RequestHandler builder.
    fn handler(
        input: Self::Input,
    ) -> Result<RequestHandler<RequestSet<Self, P>, NoOwner, NoProvider>, Self::Error> {
        let request = Self::from_input(input)?;
        let handler = RequestHandler::new().request(request);
        Ok(handler)
    }

    /// Process the request and return a reply.
    fn handle(
        self, ctx: Context<P>,
    ) -> impl Future<Output = Result<Reply<Self::Output>, Self::Error>> + Send;
}
```

## Context

Request-scoped data passed to `Handler::handle`.

```rust
#[derive(Clone, Copy, Debug)]
pub struct Context<'a, P: Provider> {
    /// Owning tenant/namespace.
    pub owner: &'a str,

    /// Provider implementation for external I/O.
    pub provider: &'a P,

    /// HTTP request headers.
    pub headers: &'a HeaderMap<String>,
}
```

## Reply

Response wrapper returned by handlers.

```rust
pub struct Reply<B: Body> {
    pub status: StatusCode,
    pub headers: HeaderMap,
    pub body: B,
}

impl<B: Body> Reply<B> {
    pub fn ok(body: B) -> Self;          // 200
    pub fn created(body: B) -> Self;     // 201
    pub fn accepted(body: B) -> Self;    // 202
    pub fn status(self, status: StatusCode) -> Self;
    pub fn headers(self, headers: HeaderMap) -> Self;
}

// Conversion: any Body automatically becomes Reply::ok
impl<B: Body> From<B> for Reply<B> {
    fn from(body: B) -> Self { Reply::ok(body) }
}
```

For messaging handlers that return no body, use `Reply::ok(())`.

## IntoBody

Trait for converting response types to HTTP-compatible bytes. Required for HTTP handler response types.

```rust
pub trait IntoBody: Body {
    fn into_body(self) -> anyhow::Result<Vec<u8>>;
}
```

Typical implementation:

```rust
impl IntoBody for MyResponse {
    fn into_body(self) -> anyhow::Result<Vec<u8>> {
        serde_json::to_vec(&self).context("serializing reply")
    }
}
```

Messaging handlers use `type Output = ()` and do not need `IntoBody`.

## Client (Typestate Builder)

Used in tests and guest wiring to invoke handlers.

```rust
// Test usage:
let client = Client::new("owner").provider(mock_provider);
let response = client.request(my_request).await?;
assert_eq!(response.status, 200);

// Guest wiring usage (via Handler::handler convenience):
MyRequest::handler(payload)?
    .provider(&Provider::new())
    .owner("at")
    .await
    .map_err(Into::into)
```

### Full Typestate Flow

```rust
// Option A: Client (preferred for tests)
let client = Client::new("owner").provider(provider);
let reply = client.request(request).await?;

// Option B: Handler::handler (preferred for guest wiring)
let reply = MyRequest::handler(input)?
    .provider(&provider)
    .owner("owner")
    .await?;

// Option C: Manual RequestHandler (rarely needed)
let reply = RequestHandler::new()
    .owner("owner")
    .provider(&provider)
    .request(request)
    .handle()
    .await?;
```

## Error Types

### Error Enum

```rust
pub enum Error {
    BadRequest { code: String, description: String },   // 400
    NotFound { code: String, description: String },     // 404
    ServerError { code: String, description: String },  // 500
    BadGateway { code: String, description: String },   // 502
}
```

Methods: `status() -> StatusCode`, `code() -> String`, `description() -> String`.

### Error Macros

```rust
bad_request!("invalid input");
bad_request!("field {} is required", field_name);
server_error!("internal failure");
bad_gateway!("upstream API failed: {e}");
```

### Error Conversion

`anyhow::Error` automatically converts to `Error::ServerError` unless the inner error is already an `Error` variant (in which case it preserves the variant and appends context).

```rust
// anyhow context preserved:
Err(bad_request!("invalid")).context("parsing request")?;
// Result: BadRequest { code: "bad_request", description: "parsing request: ..." }
```

### Result Type

```rust
pub type Result<T> = anyhow::Result<T, Error>;
```

## HttpResult and HttpError

Used in guest wiring handler functions for Axum compatibility:

```rust
pub type HttpResult<T: IntoResponse, E: IntoResponse = HttpError> = Result<T, E>;

// Guest handler returns:
async fn my_handler(body: Bytes) -> HttpResult<Reply<MyResponse>> { ... }
```

`HttpError` converts from both `omnia_sdk::Error` and `anyhow::Error`.

## Re-exports

The SDK re-exports commonly needed crates:

```rust
pub use {anyhow, axum, bytes, http, http_body, tracing};
```

On `wasm32`:

```rust
pub use {omnia_wasi_http, omnia_wasi_identity, omnia_wasi_keyvalue,
         omnia_wasi_messaging, omnia_wasi_otel, wasip3, wit_bindgen};
```

## ensure_env! Macro

Used in the guest Provider to verify required environment variables at initialization:

```rust
omnia_sdk::ensure_env!("API_URL", "AZURE_IDENTITY");
```

Panics with a descriptive message listing all missing variables.
