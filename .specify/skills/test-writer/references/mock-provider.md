# MockProvider Patterns

Two production-proven patterns for mocking omnia SDK provider traits in tests.

## Pattern A: Static MockProvider (cars)

For crates that call external HTTP APIs without time-sensitive validation. Records outgoing requests for assertion.

```rust
use std::any::Any;
use std::error::Error;
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result, anyhow};
use bytes::Bytes;
use cars::{Config, HttpRequest};
use http::{Request, Response};
use percent_encoding::percent_decode_str;
use serde_json::Value;

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

    /// Get recorded requests filtered by URI path.
    #[must_use]
    pub fn requests_for(&self, path: &str) -> Vec<RecordedRequest> {
        self.requests().into_iter().filter(|record| record.path == path).collect()
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
        Response::builder().status(200).body(body).context("building response")
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

    /// Decode the `filter=` query parameter into a JSON Value.
    #[must_use]
    pub fn filter(&self) -> Option<Value> {
        let query = self.query.as_ref()?;
        let encoded = query.split('&').find_map(|pair| pair.strip_prefix("filter="))?;
        let decoded = percent_decode_str(encoded).decode_utf8().ok()?;
        serde_json::from_str::<Value>(&decoded).ok()
    }
}
```

### Key Design Points

- `Arc<Mutex<Vec<RecordedRequest>>>` captures all HTTP requests made during the test
- `include_bytes!` loads JSON fixture files at compile time (stored in `tests/data/`)
- Dispatch on `request.uri().path()` to return different fixture data per endpoint
- `RecordedRequest::filter()` decodes URL-encoded filter parameters for assertion
- `Config::get` returns test values; errors for unknown keys catch misconfigured tests

## Pattern B: Replay MockProvider (r9k-adapter)

For crates with time-sensitive validation. Uses `augentic-test` framework for HTTP mocking and event capture.

```rust
use core::panic;
use std::any::Any;
use std::error::Error;
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result, anyhow};
use augentic_test::{Fetcher, Fixture, PreparedTestCase, TestDef, TestResult};
use bytes::Bytes;
use chrono::{Timelike, Utc};
use chrono_tz::Pacific::Auckland;
use http::{Request, Response};
use omnia_sdk::{Config, HttpRequest, Identity, Message, Publish};
use r9k_adapter::{R9kMessage, SmarTrakEvent};
use serde::Deserialize;

// --- Replay Fixture (implements augentic_test::Fixture) ---

#[derive(Debug, Clone, Deserialize)]
pub struct Replay {
    pub input: Option<R9kMessage>,
    pub params: Option<ReplayTransform>,
    pub output: Option<ReplayOutput>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum ReplayOutput {
    Events(Vec<SmarTrakEvent>),
    Error(omnia_sdk::Error),
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct ReplayTransform {
    pub delay: i32,
}

impl Fixture for Replay {
    type Error = omnia_sdk::Error;
    type Input = R9kMessage;
    type Output = Vec<SmarTrakEvent>;
    type TransformParams = ReplayTransform;

    fn from_data(data_def: &TestDef<Self::Error>) -> Self {
        let input_str: Option<String> = data_def.input.as_ref().and_then(|v| {
            serde_json::from_value(v.clone()).expect("deserialize input as XML String")
        });
        let input = input_str.map(|s| {
            quick_xml::de::from_str(&s).expect("deserialize R9kMessage")
        });
        let params: Option<Self::TransformParams> = data_def.params.as_ref().and_then(|v| {
            serde_json::from_value(v.clone()).expect("deserialize transform parameters")
        });
        let Some(output_def) = &data_def.output else {
            return Self { input, params, output: None };
        };
        let output = match output_def {
            TestResult::Success(value) => serde_json::from_value(value.clone()).map_or_else(
                |_| panic!("deserialize output as SmarTrak events"),
                |events| Some(ReplayOutput::Events(events)),
            ),
            TestResult::Failure(err) => Some(ReplayOutput::Error(err.clone())),
        };
        Self { input, params, output }
    }

    fn input(&self) -> Option<Self::Input> { self.input.clone() }
    fn params(&self) -> Option<Self::TransformParams> { self.params.clone() }

    fn output(&self) -> Option<Result<Self::Output, Self::Error>> {
        let output = self.output.as_ref()?;
        match output {
            ReplayOutput::Error(error) => Some(Err(error.clone())),
            ReplayOutput::Events(events) => {
                if events.is_empty() { return None; }
                Some(Ok(events.clone()))
            }
        }
    }
}

// --- MockProvider ---

#[derive(Clone)]
pub struct MockProvider {
    test_case: PreparedTestCase<Replay>,
    events: Arc<Mutex<Vec<SmarTrakEvent>>>,
}

impl MockProvider {
    #[must_use]
    pub fn new(test_case: PreparedTestCase<Replay>) -> Self {
        Self { test_case, events: Arc::new(Mutex::new(Vec::new())) }
    }

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
        let Some(http_requests) = &self.test_case.http_requests else {
            return Err(anyhow!("no http requests defined in replay session"));
        };
        let fetcher = Fetcher::new(http_requests);
        fetcher.fetch(&request)
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

// --- Time-shifting function ---

#[must_use]
pub fn shift_time(input: &R9kMessage, params: Option<&ReplayTransform>) -> R9kMessage {
    if params.is_none() {
        return input.clone();
    }
    let delay = params.as_ref().map_or(0, |p| p.delay);
    let mut request = input.clone();
    let Some(change) = request.train_update.changes.get_mut(0) else {
        return request;
    };

    let now = Utc::now().with_timezone(&Auckland);
    request.train_update.created_date = now.date_naive();

    #[allow(clippy::cast_possible_wrap)]
    let from_midnight = now.num_seconds_from_midnight() as i32;
    let adjusted_secs = from_midnight - delay;

    if change.has_departed {
        change.actual_departure_time = adjusted_secs;
    } else if change.has_arrived {
        change.actual_arrival_time = adjusted_secs;
    }
    request
}
```

### Key Design Points

- `Replay` struct implements `augentic_test::Fixture` with 4 associated types
- `ReplayOutput` is `#[serde(untagged)]` to handle both events and error variants
- `MockProvider` holds a `PreparedTestCase<Replay>` with HTTP mock data
- `HttpRequest::fetch` delegates to `Fetcher` which matches on authority/method/path
- `Publish::send` deserializes message payload and captures events for assertion
- `shift_time` adjusts timestamps so time-sensitive validation passes at any clock time

## Trait Implementation Reference

For complete MockProvider implementations of each trait (basic and advanced patterns), see the canonical provider references:

- [Config](providers/config.md#mockprovider-implementation) -- match on key, error on unknown
- [HttpRequest](providers/http-request.md#mockprovider-implementation) -- URI pattern matching, embedded test data
- [Publish](providers/publish.md#mockprovider-implementation) -- event capture with Arc<Mutex<Vec<Message>>>
- [Identity](providers/identity.md#mockprovider-implementation) -- mock token return, request tracking
- [StateStore](providers/state-store.md#mockprovider-implementation) -- OnceCell + Mutex, TTL tracking, cache verification helpers
- [Broadcast](providers/broadcast.md#mockprovider-implementation) -- capture sends with channel and target info
- [TableStore](../../crate-writer/examples/capabilities/tablestore.md) -- return fixture rows from `query`, affected count from `exec`
- [Multi-trait MockProvider](providers/README.md#multi-trait-mockprovider) -- combining all traits in a single provider
