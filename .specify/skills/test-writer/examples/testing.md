# Testing Patterns

How to generate tests for Omnia crates. Tests use a `MockProvider` that implements the required capability traits, and the `Client` typestate builder to invoke handlers.

## Test Directory Structure

```
$CRATE_PATH/
├── tests/
│   ├── provider.rs         # MockProvider (shared across tests)
│   ├── <handler_a>.rs      # Tests for handler A
│   └── <handler_b>.rs      # Tests for handler B
└── tests/data/             # JSON/XML fixture files (optional)
    ├── response-a.json
    └── response-b.json
```

Each test file includes the provider module:

```rust
mod provider;
```

## MockProvider: HTTP API Crate (cars pattern)

For crates that primarily call external HTTP APIs. Records requests and returns fixture data.

```rust
use std::any::Any;
use std::error::Error;
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result, anyhow};
use bytes::Bytes;
use cars::{Config, HttpRequest};
use http::{Request, Response};

#[derive(Clone, Default)]
pub struct MockProvider {
    requests: Arc<Mutex<Vec<RecordedRequest>>>,
}

impl MockProvider {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Get all recorded HTTP requests.
    #[must_use]
    pub fn requests(&self) -> Vec<RecordedRequest> {
        self.requests.lock().expect("requests mutex poisoned").clone()
    }

    /// Get recorded requests filtered by path.
    #[must_use]
    pub fn requests_for(&self, path: &str) -> Vec<RecordedRequest> {
        self.requests().into_iter().filter(|r| r.path == path).collect()
    }

    fn record_request<B>(&self, request: &Request<B>) {
        let record = RecordedRequest::from(request);
        self.requests.lock().expect("requests mutex poisoned").push(record);
    }
}

impl Config for MockProvider {
    async fn get(&self, key: &str) -> Result<String> {
        Ok(match key {
            "MWS_API_KEY" => "test_api_key".to_string(),
            _ => return Err(anyhow!("unknown config key: {key}")),
        })
    }
}

impl HttpRequest for MockProvider {
    async fn fetch<T>(&self, request: Request<T>) -> Result<Response<Bytes>>
    where
        T: http_body::Body + Any,
        T::Data: Into<Vec<u8>>,
        T::Error: Into<Box<dyn Error + Send + Sync + 'static>>,
    {
        self.record_request(&request);

        // Dispatch on URI path to return appropriate fixture data
        let data = match request.uri().path() {
            "/v1/prod/worksite-search" => {
                include_bytes!("data/worksite-search.json").as_slice()
            }
            "/v1/prod/tmp-search" => {
                include_bytes!("data/tmp-search.json").as_slice()
            }
            "/v1/prod/layouts" => {
                include_bytes!("data/layouts.json").as_slice()
            }
            _ => {
                return Err(anyhow!("unknown path: {}", request.uri().path()));
            }
        };

        let body = Bytes::from(data);
        Response::builder().status(200).body(body).context("failed to build response")
    }
}

/// Recorded HTTP request for test assertions.
#[derive(Clone, Debug)]
pub struct RecordedRequest {
    pub path: String,
    pub query: Option<String>,
}

impl RecordedRequest {
    fn from<B>(request: &Request<B>) -> Self {
        Self {
            path: request.uri().path().to_string(),
            query: request.uri().query().map(ToString::to_string),
        }
    }
}
```

### Key Patterns

- `Arc<Mutex<Vec<RecordedRequest>>>` to capture HTTP requests made during test
- `include_bytes!` for loading JSON fixture files at compile time
- Dispatch on `request.uri().path()` to return different fixture data
- `Config::get` returns hardcoded test values or errors for unknown keys
- `RecordedRequest` struct for inspecting what API calls were made

## MockProvider: Messaging Crate (r9k-adapter pattern)

For crates that publish events. Captures published messages for assertion.

```rust
use std::any::Any;
use std::error::Error;
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result, anyhow};
use bytes::Bytes;
use http::{Request, Response};
use omnia_sdk::{Config, HttpRequest, Identity, Message, Publish};
use r9k_adapter::SmarTrakEvent;

#[derive(Clone)]
pub struct MockProvider {
    events: Arc<Mutex<Vec<SmarTrakEvent>>>,
}

impl MockProvider {
    #[must_use]
    pub fn new() -> Self {
        Self { events: Arc::new(Mutex::new(Vec::new())) }
    }

    /// Get all published events.
    #[must_use]
    pub fn events(&self) -> Vec<SmarTrakEvent> {
        self.events.lock().expect("should lock").clone()
    }
}

impl Config for MockProvider {
    async fn get(&self, _key: &str) -> Result<String> {
        Ok("http://localhost:8080".to_string())
    }
}

impl HttpRequest for MockProvider {
    async fn fetch<T>(&self, request: Request<T>) -> Result<Response<Bytes>>
    where
        T: http_body::Body + Any,
        T::Data: Into<Vec<u8>>,
        T::Error: Into<Box<dyn Error + Send + Sync + 'static>>,
    {
        // Return fixture data based on the request URI
        let data = match request.uri().path() {
            path if path.contains("allocations") => {
                br#"["vehicle1", "vehicle2"]"#.as_slice()
            }
            path if path.contains("stops") => {
                include_bytes!("../data/stops.json").as_slice()
            }
            _ => return Err(anyhow!("unknown path: {}", request.uri().path())),
        };

        let body = Bytes::from(data);
        Response::builder().status(200).body(body).context("building response")
    }
}

impl Publish for MockProvider {
    async fn send(&self, _topic: &str, message: &Message) -> Result<()> {
        let event: SmarTrakEvent =
            serde_json::from_slice(&message.payload).context("deserializing event")?;
        self.events.lock().map_err(|e| anyhow!("{e}"))?.push(event);
        Ok(())
    }
}

impl Identity for MockProvider {
    async fn access_token(&self, _identity: String) -> Result<String> {
        Ok("mock_access_token".to_string())
    }
}
```

### Key Patterns

- `Publish::send` captures events by deserializing the message payload
- `Identity::access_token` returns a mock token
- HTTP fixture data can be inline (`br#"..."#`) or from files (`include_bytes!`)

## Test Structure: Happy Path

```rust
mod provider;

use cars::FeatureRequest;
use provider::MockProvider;
use omnia_sdk::api::Client;

#[tokio::test]
async fn single_feature() {
    let provider = MockProvider::new();
    let client = Client::new("owner").provider(provider.clone());

    let request = FeatureRequest { id: "899".to_string() };
    let response = client.request(request).await.expect("request should succeed");

    assert_eq!(response.status, 200);

    let feature = response.body.0;
    assert_eq!(feature.properties.worksite_code, "AT-1");
    assert_eq!(feature.properties.work_status, "READY_TO_START");
}
```

## Test Structure: Verify API Calls

Assert what HTTP calls the handler made and what filters it used.

```rust
#[tokio::test]
async fn verify_filter() {
    let provider = MockProvider::new();
    let client = Client::new("owner").provider(provider.clone());

    let request = FeatureRequest { id: "899".to_string() };
    client.request(request).await.expect("request should succeed");

    // Verify the HTTP call was made
    let calls = provider.requests_for("/v1/prod/worksite-search");
    assert_eq!(calls.len(), 1, "expected single worksite-search call");

    // Verify the filter parameter
    let filter = calls[0].filter().expect("filter should be present");
    let worksite_id = filter
        .get("where")
        .and_then(|w| w.get("worksiteId"))
        .and_then(|w| w.get("eq"))
        .and_then(|v| v.as_str());
    assert_eq!(worksite_id, Some("899"));
}
```

## Test Structure: Optional Related Data

Test that optional data fetching is controlled by request parameters.

```rust
#[tokio::test]
async fn without_tmps() {
    let provider = MockProvider::new();
    let client = Client::new("owner").provider(provider.clone());

    let request = WorksiteRequest {
        worksite_code: "AT-1".to_string(),
        include_tmps: Some(false),  // explicitly skip TMPs
        date_from: None,
        date_to: None,
    };

    let response = client.request(request).await.expect("should succeed");
    assert!(response.body.0.tmps.is_none());

    // Verify TMP search was NOT called
    let tmp_calls = provider.requests_for("/v1/prod/tmp-search");
    assert!(tmp_calls.is_empty(), "tmp-search should not be called");
}
```

## Test Structure: Error Cases

Test that handlers return expected errors for invalid input.

```rust
#[tokio::test]
async fn no_changes() {
    let provider = MockProvider::new();
    let client = Client::new("at").provider(provider.clone());

    let message = R9kMessage { train_update: TrainUpdate::default() };

    let error = client.request(message).await.expect_err("should have error");
    assert_eq!(error.code(), "no_update");
    assert_eq!(error.description(), "contains no updates");
}
```

## Test Structure: Published Events

For messaging handlers, assert on the events that were published.

```rust
#[tokio::test]
async fn arrival_event() {
    let provider = MockProvider::new();
    let client = Client::new("at").provider(provider.clone());

    let message = build_test_message(/* ... */);
    client.request(message).await.expect("should process");

    let events = provider.events();
    assert_eq!(events.len(), 2);  // published 2x for departure signaling

    let event = &events[0];
    assert_eq!(event.event_type, EventType::Location);
    assert!(event.location_data.latitude.eq(&-36.12345));
    assert_eq!(event.remote_data.external_id, "vehicle1");
}
```

## Test Fixture Files

Store JSON response data in `tests/data/`:

```
tests/data/
├── worksite-search.json     # Mock response for worksite search
├── tmp-search.json          # Mock response for TMP search
├── layouts.json             # Mock response for layouts
└── feature-collection.json  # Mock response for GIS features
```

Reference in MockProvider with `include_bytes!`:

```rust
"/v1/prod/worksite-search" => include_bytes!("data/worksite-search.json").as_slice(),
```

## Summary of Test Conventions

1. **Each test file** starts with `mod provider;`
2. **Create provider** with `MockProvider::new()`
3. **Create client** with `Client::new("owner").provider(provider.clone())`
4. **Invoke handler** with `client.request(request).await`
5. **Assert on response**: `response.status`, `response.body`
6. **Assert on side effects**: `provider.events()`, `provider.requests_for(path)`
7. **Error testing**: `.expect_err("message")` then assert `error.code()` and `error.description()`
8. **Async runtime**: `#[tokio::test]`
