use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use http::Request;
use http::header::HOST;
use http_body_util::BodyExt;
use http_body_util::combinators::UnsyncBoxBody;
use wasmtime_wasi_http::p3::bindings::http::types::ErrorCode;

use super::circuit_breaker::{BreakerDecision, BucketRegistry};
use super::retry::{RetryPolicy, retry_send};

/// Header names used to carry per-request resilience policy across the WASI boundary.
const HEADER_TIMEOUT_MS: &str = "x-omnia-timeout-ms";
const HEADER_UPSTREAM: &str = "x-omnia-upstream";

/// Resilience configuration: retry policy + circuit breaker registry.
///
/// When `Some` on `HttpHooks`, retry and circuit breaker are active.
/// When `None`, requests are sent directly with only timeout applied.
#[derive(Debug, Clone)]
pub struct ResilienceConfig {
    pub retry_max: u8,
    pub retry_policy: RetryPolicy,
    pub registry: Arc<BucketRegistry>,
}

/// Send an outbound HTTP request through the full pipeline.
///
/// Strips `X-Omnia-*` headers, resolves timeout, collects the body, then
/// branches based on whether resilience is enabled:
/// - `Some(config)` — circuit breaker check, retry loop, breaker recording
/// - `None` — direct send with optional timeout
pub async fn send(
    client: &reqwest::Client, request: Request<UnsyncBoxBody<Bytes, ErrorCode>>,
    resilience: Option<&ResilienceConfig>, default_timeout: Duration,
) -> Result<reqwest::Response, ErrorCode> {
    let (mut parts, body) = request.into_parts();

    // Extract and strip resilience headers (always, even when disabled)
    let timeout_ms = parts
        .headers
        .remove(HEADER_TIMEOUT_MS)
        .and_then(|v| v.to_str().ok().and_then(|s| s.parse::<u64>().ok()));
    let upstream =
        parts.headers.remove(HEADER_UPSTREAM).and_then(|v| v.to_str().ok().map(String::from));

    parts.headers.remove(HOST);

    let timeout = match timeout_ms {
        Some(0) => None,
        Some(ms) => Some(Duration::from_millis(ms)),
        None if default_timeout.is_zero() => None,
        None => Some(default_timeout),
    };

    // Collect body (needed for both paths — retries clone from bytes)
    let collected =
        body.collect().await.map_err(|e| ErrorCode::InternalError(Some(e.to_string())))?;
    let body_bytes = collected.to_bytes();

    let url = parts.uri.to_string();

    match resilience {
        Some(config) => {
            send_with_resilience(client, config, &parts, &url, body_bytes, upstream, timeout).await
        }
        None => send_direct(client, &parts, &url, body_bytes, timeout).await,
    }
}

/// Direct send — no retry, no circuit breaker.
async fn send_direct(
    client: &reqwest::Client, parts: &http::request::Parts, url: &str, body: Bytes,
    timeout: Option<Duration>,
) -> Result<reqwest::Response, ErrorCode> {
    let mut builder =
        client.request(parts.method.clone(), url).headers(parts.headers.clone()).body(body);
    if let Some(t) = timeout {
        builder = builder.timeout(t);
    }
    builder.send().await.map_err(reqwest_to_error_code)
}

/// Send with retry and optional circuit breaker.
///
/// The breaker only activates when the guest explicitly names a bucket via
/// `x-omnia-upstream`. Unknown bucket names are a misconfiguration error.
/// Requests without an upstream still get retry + timeout.
async fn send_with_resilience(
    client: &reqwest::Client, config: &ResilienceConfig, parts: &http::request::Parts, url: &str,
    body: Bytes, upstream: Option<String>, timeout: Option<Duration>,
) -> Result<reqwest::Response, ErrorCode> {
    let resolved = config
        .registry
        .resolve(upstream.as_deref())
        .map_err(|msg| ErrorCode::InternalError(Some(msg)))?;

    if let Some(ref r) = resolved
        && r.breaker.check() == BreakerDecision::Reject
    {
        tracing::warn!(bucket = %r.bucket, url = %url, "outbound request blocked by circuit breaker");
        return Err(ErrorCode::InternalError(Some(format!("circuit breaker open: {}", r.bucket))));
    }

    let result = retry_send(
        client,
        &parts.method,
        url,
        parts.headers.clone(),
        body,
        config.retry_max,
        &config.retry_policy,
        timeout,
    )
    .await;

    if let Some(ref r) = resolved {
        if is_breaker_fault(&result.as_ref()) {
            r.breaker.record_failure();
        } else {
            r.breaker.record_success();
        }
    }

    result.map_err(reqwest_to_error_code)
}

/// Determines whether an outcome should count as a circuit breaker fault.
///
/// **Fault** (counts toward tripping / re-trips in `HalfOpen`):
///   - 5xx server errors
///   - Connect errors (connection refused, DNS failure, etc.)
///   - Timeout errors
///
/// **Success** (counts toward `HalfOpen` → `Closed` recovery):
///   - Any completed response that is not a fault, including 4xx client errors
///   - 429 (Too Many Requests) is intentionally treated as success — it's a
///     rate-limiting signal, not upstream degradation. The retry layer already
///     handles 429 with `Retry-After` support.
///   - All other transport errors (decode, redirect, body stream) are also
///     treated as non-faults — they indicate client-side issues, not upstream
///     degradation.
fn is_breaker_fault(result: &Result<&reqwest::Response, &reqwest::Error>) -> bool {
    match result {
        Err(e) => e.is_timeout() || e.is_connect(),
        Ok(resp) => resp.status().is_server_error(),
    }
}

#[allow(clippy::needless_pass_by_value)]
pub(super) fn reqwest_to_error_code(e: reqwest::Error) -> ErrorCode {
    if e.is_timeout() {
        ErrorCode::ConnectionTimeout
    } else if e.is_connect() {
        ErrorCode::ConnectionRefused
    } else if e.is_request() {
        ErrorCode::HttpRequestUriInvalid
    } else {
        ErrorCode::InternalError(Some(e.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    use http::Request;
    use http_body_util::Empty;
    use wiremock::matchers::method;
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;
    use crate::host::circuit_breaker::{BreakerConfig, BucketRegistry, State};

    fn test_client() -> reqwest::Client {
        reqwest::Client::builder().no_proxy().build().unwrap()
    }

    const DEFAULT_TIMEOUT: Duration = Duration::from_secs(5);

    fn breaker_config() -> BreakerConfig {
        BreakerConfig {
            trip_threshold: 3,
            recovery_threshold: 2,
            reset_period: Duration::from_millis(100),
            fault_window: Duration::from_secs(5),
        }
    }

    const TEST_BUCKET: &str = "test";

    fn test_config() -> ResilienceConfig {
        test_config_with_buckets(TEST_BUCKET)
    }

    fn test_config_with_buckets(names: &str) -> ResilienceConfig {
        let bucket_names = names.split(',').map(|s| s.trim().to_owned()).filter(|s| !s.is_empty());
        ResilienceConfig {
            retry_max: 2,
            retry_policy: RetryPolicy {
                base_delay_ms: 10,
                cap_delay_ms: 50,
            },
            registry: Arc::new(BucketRegistry::new(bucket_names, &breaker_config()).unwrap()),
        }
    }

    fn resolve_breaker(
        config: &ResilienceConfig, name: &str,
    ) -> Arc<crate::host::circuit_breaker::CircuitBreaker> {
        config.registry.resolve(Some(name)).unwrap().unwrap().breaker
    }

    fn get_request(uri: &str) -> Request<UnsyncBoxBody<Bytes, ErrorCode>> {
        Request::get(uri)
            .body(
                Empty::new()
                    .map_err(|e| ErrorCode::InternalError(Some(e.to_string())))
                    .boxed_unsync(),
            )
            .unwrap()
    }

    fn get_request_with_headers(
        uri: &str, timeout_ms: Option<u64>, upstream: Option<&str>,
    ) -> Request<UnsyncBoxBody<Bytes, ErrorCode>> {
        let mut builder = Request::get(uri);
        if let Some(ms) = timeout_ms {
            builder = builder.header(HEADER_TIMEOUT_MS, ms.to_string());
        }
        if let Some(name) = upstream {
            builder = builder.header(HEADER_UPSTREAM, name);
        }
        builder
            .body(
                Empty::new()
                    .map_err(|e| ErrorCode::InternalError(Some(e.to_string())))
                    .boxed_unsync(),
            )
            .unwrap()
    }

    fn post_request(uri: &str) -> Request<UnsyncBoxBody<Bytes, ErrorCode>> {
        Request::post(uri)
            .body(
                Empty::new()
                    .map_err(|e| ErrorCode::InternalError(Some(e.to_string())))
                    .boxed_unsync(),
            )
            .unwrap()
    }

    #[tokio::test]
    async fn timeout_applied_from_header() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_delay(Duration::from_secs(2)))
            .mount(&server)
            .await;

        let req = get_request_with_headers(&server.uri(), Some(500), None);
        let config = test_config();
        let result = send(&test_client(), req, Some(&config), DEFAULT_TIMEOUT).await;

        result.unwrap_err();
    }

    #[tokio::test]
    async fn default_timeout_when_no_header() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200))
            .expect(1)
            .mount(&server)
            .await;

        let req = get_request(&server.uri());
        let config = test_config();
        let result = send(&test_client(), req, Some(&config), DEFAULT_TIMEOUT).await;

        result.unwrap();
    }

    #[tokio::test]
    async fn upstream_header_selects_bucket() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200))
            .expect(1)
            .mount(&server)
            .await;

        let config = test_config_with_buckets("monitoring");
        let req = get_request_with_headers(&server.uri(), None, Some("monitoring"));
        let result = send(&test_client(), req, Some(&config), DEFAULT_TIMEOUT).await;

        result.unwrap();
    }

    #[tokio::test]
    async fn unknown_bucket_returns_error() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200))
            .expect(0)
            .mount(&server)
            .await;

        let config = test_config_with_buckets("monitoring");
        let req = get_request_with_headers(&server.uri(), None, Some("nonexistent"));
        let err = send(&test_client(), req, Some(&config), DEFAULT_TIMEOUT).await.unwrap_err();

        match err {
            ErrorCode::InternalError(Some(msg)) => {
                assert!(msg.contains("unknown circuit breaker bucket"), "unexpected: {msg}");
            }
            other => panic!("expected InternalError, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn no_upstream_skips_breaker() {
        let server = MockServer::start().await;
        Mock::given(method("GET")).respond_with(ResponseTemplate::new(503)).mount(&server).await;

        let config = ResilienceConfig {
            retry_max: 0,
            ..test_config()
        };

        // Send many 503s without specifying an upstream — breaker should not activate
        for _ in 0..5 {
            let req = get_request(&server.uri());
            let result = send(&test_client(), req, Some(&config), DEFAULT_TIMEOUT).await;
            assert_eq!(result.unwrap().status(), 503);
        }
    }

    #[tokio::test]
    async fn no_upstream_still_retries() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(503))
            .up_to_n_times(2)
            .mount(&server)
            .await;
        Mock::given(method("GET")).respond_with(ResponseTemplate::new(200)).mount(&server).await;

        let config = test_config();
        let req = get_request(&server.uri());
        let result = send(&test_client(), req, Some(&config), DEFAULT_TIMEOUT).await;

        result.unwrap();
        let received = server.received_requests().await.unwrap();
        assert_eq!(received.len(), 3, "should retry twice then succeed without breaker");
    }

    #[tokio::test]
    async fn no_upstream_still_applies_timeout() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_delay(Duration::from_secs(2)))
            .mount(&server)
            .await;

        let config = ResilienceConfig {
            retry_max: 0,
            ..test_config()
        };
        let req = get_request_with_headers(&server.uri(), Some(500), None);
        let result = send(&test_client(), req, Some(&config), DEFAULT_TIMEOUT).await;

        result.unwrap_err();
    }

    #[tokio::test]
    async fn headers_stripped_before_forwarding() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200))
            .expect(1)
            .mount(&server)
            .await;

        let req = get_request_with_headers(&server.uri(), Some(5000), Some("monitoring"));
        let config = test_config_with_buckets("monitoring");
        let result = send(&test_client(), req, Some(&config), DEFAULT_TIMEOUT).await;

        result.unwrap();

        let received = server.received_requests().await.unwrap();
        assert_eq!(received.len(), 1);
        assert!(received[0].headers.get("x-omnia-timeout-ms").is_none());
        assert!(received[0].headers.get("x-omnia-upstream").is_none());
    }

    #[tokio::test]
    async fn retry_inside_breaker() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(503))
            .up_to_n_times(2)
            .mount(&server)
            .await;
        Mock::given(method("GET")).respond_with(ResponseTemplate::new(200)).mount(&server).await;

        let config = test_config();
        let req = get_request_with_headers(&server.uri(), None, Some(TEST_BUCKET));
        let result = send(&test_client(), req, Some(&config), DEFAULT_TIMEOUT).await;

        result.unwrap();
        assert_eq!(resolve_breaker(&config, TEST_BUCKET).state(), State::Closed);
    }

    #[tokio::test]
    async fn open_breaker_returns_error_immediately() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200))
            .expect(0)
            .mount(&server)
            .await;

        let config = test_config();
        let breaker = resolve_breaker(&config, TEST_BUCKET);
        for _ in 0..3 {
            breaker.record_failure();
        }
        assert_eq!(breaker.state(), State::Open);

        let req = get_request_with_headers(&server.uri(), None, Some(TEST_BUCKET));
        let result = send(&test_client(), req, Some(&config), DEFAULT_TIMEOUT).await;

        result.unwrap_err();
    }

    #[tokio::test]
    async fn open_breaker_error_code() {
        let server = MockServer::start().await;
        let config = test_config();
        let breaker = resolve_breaker(&config, TEST_BUCKET);
        for _ in 0..3 {
            breaker.record_failure();
        }

        let req = get_request_with_headers(&server.uri(), None, Some(TEST_BUCKET));
        let err = send(&test_client(), req, Some(&config), DEFAULT_TIMEOUT).await.unwrap_err();

        match err {
            ErrorCode::InternalError(Some(msg)) => {
                assert!(msg.contains("circuit breaker open"), "unexpected message: {msg}");
            }
            other => panic!("expected InternalError, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn breaker_opens_after_threshold_failures() {
        let server = MockServer::start().await;
        Mock::given(method("GET")).respond_with(ResponseTemplate::new(503)).mount(&server).await;

        let config = ResilienceConfig {
            retry_max: 0,
            ..test_config()
        };

        for _ in 0..3 {
            let req = get_request_with_headers(&server.uri(), None, Some(TEST_BUCKET));
            let _ = send(&test_client(), req, Some(&config), DEFAULT_TIMEOUT).await;
        }

        assert_eq!(resolve_breaker(&config, TEST_BUCKET).state(), State::Open);

        let req = get_request_with_headers(&server.uri(), None, Some(TEST_BUCKET));
        let result = send(&test_client(), req, Some(&config), DEFAULT_TIMEOUT).await;
        result.unwrap_err();
    }

    #[tokio::test]
    async fn breaker_recovers_via_half_open() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(503))
            .up_to_n_times(3)
            .mount(&server)
            .await;
        Mock::given(method("GET")).respond_with(ResponseTemplate::new(200)).mount(&server).await;

        let config = ResilienceConfig {
            retry_max: 0,
            retry_policy: RetryPolicy {
                base_delay_ms: 10,
                cap_delay_ms: 50,
            },
            registry: Arc::new(
                BucketRegistry::new(
                    [TEST_BUCKET.to_owned()],
                    &BreakerConfig {
                        trip_threshold: 3,
                        recovery_threshold: 2,
                        reset_period: Duration::from_millis(50),
                        fault_window: Duration::from_secs(5),
                    },
                )
                .unwrap(),
            ),
        };

        for _ in 0..3 {
            let req = get_request_with_headers(&server.uri(), None, Some(TEST_BUCKET));
            let _ = send(&test_client(), req, Some(&config), DEFAULT_TIMEOUT).await;
        }
        assert_eq!(resolve_breaker(&config, TEST_BUCKET).state(), State::Open);

        tokio::time::sleep(Duration::from_millis(100)).await;

        let req = get_request_with_headers(&server.uri(), None, Some(TEST_BUCKET));
        let result = send(&test_client(), req, Some(&config), DEFAULT_TIMEOUT).await;
        result.unwrap();

        let req = get_request_with_headers(&server.uri(), None, Some(TEST_BUCKET));
        let result = send(&test_client(), req, Some(&config), DEFAULT_TIMEOUT).await;
        result.unwrap();
        assert_eq!(resolve_breaker(&config, TEST_BUCKET).state(), State::Closed);
    }

    #[tokio::test]
    async fn per_bucket_isolation() {
        let server_a = MockServer::start().await;
        let server_b = MockServer::start().await;
        Mock::given(method("GET")).respond_with(ResponseTemplate::new(503)).mount(&server_a).await;
        Mock::given(method("GET")).respond_with(ResponseTemplate::new(200)).mount(&server_b).await;

        let config = ResilienceConfig {
            retry_max: 0,
            ..test_config_with_buckets("a,b")
        };

        for _ in 0..3 {
            let req = get_request_with_headers(&server_a.uri(), None, Some("a"));
            let _ = send(&test_client(), req, Some(&config), DEFAULT_TIMEOUT).await;
        }

        let req = get_request_with_headers(&server_b.uri(), None, Some("b"));
        let result = send(&test_client(), req, Some(&config), DEFAULT_TIMEOUT).await;
        result.unwrap();
    }

    #[tokio::test]
    async fn non_retryable_method_still_uses_breaker() {
        let server = MockServer::start().await;
        Mock::given(method("POST")).respond_with(ResponseTemplate::new(503)).mount(&server).await;

        let config = test_config();

        for _ in 0..3 {
            let mut req = post_request(&server.uri());
            req.headers_mut().insert(HEADER_UPSTREAM, TEST_BUCKET.parse().unwrap());
            let _ = send(&test_client(), req, Some(&config), DEFAULT_TIMEOUT).await;
        }

        assert_eq!(resolve_breaker(&config, TEST_BUCKET).state(), State::Open);
    }

    #[tokio::test]
    async fn breaker_does_not_fault_on_4xx() {
        let server = MockServer::start().await;
        Mock::given(method("GET")).respond_with(ResponseTemplate::new(404)).mount(&server).await;

        let config = ResilienceConfig {
            retry_max: 0,
            ..test_config()
        };

        for _ in 0..5 {
            let req = get_request_with_headers(&server.uri(), None, Some(TEST_BUCKET));
            let _ = send(&test_client(), req, Some(&config), DEFAULT_TIMEOUT).await;
        }

        assert_eq!(resolve_breaker(&config, TEST_BUCKET).state(), State::Closed);
    }

    #[tokio::test]
    async fn successful_retry_does_not_fault_breaker() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(503))
            .up_to_n_times(1)
            .mount(&server)
            .await;
        Mock::given(method("GET")).respond_with(ResponseTemplate::new(200)).mount(&server).await;

        let config = test_config();
        let req = get_request_with_headers(&server.uri(), None, Some(TEST_BUCKET));
        let result = send(&test_client(), req, Some(&config), DEFAULT_TIMEOUT).await;

        result.unwrap();
        assert_eq!(resolve_breaker(&config, TEST_BUCKET).state(), State::Closed);
    }

    #[tokio::test]
    async fn timeout_budget_shared_across_retries() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(503).set_delay(Duration::from_secs(2)))
            .mount(&server)
            .await;

        let config = ResilienceConfig {
            retry_max: 5,
            retry_policy: RetryPolicy {
                base_delay_ms: 10,
                cap_delay_ms: 50,
            },
            registry: Arc::new(
                BucketRegistry::new([TEST_BUCKET.to_owned()], &breaker_config()).unwrap(),
            ),
        };

        let start = Instant::now();
        let req = get_request_with_headers(&server.uri(), None, Some(TEST_BUCKET));
        let _ = send(&test_client(), req, Some(&config), Duration::from_secs(3)).await;

        assert!(start.elapsed() < Duration::from_secs(6));
    }

    #[tokio::test]
    async fn four29_does_not_fault_breaker() {
        let server = MockServer::start().await;
        Mock::given(method("GET")).respond_with(ResponseTemplate::new(429)).mount(&server).await;

        let config = ResilienceConfig {
            retry_max: 0,
            ..test_config()
        };

        for _ in 0..5 {
            let req = get_request_with_headers(&server.uri(), None, Some(TEST_BUCKET));
            let _ = send(&test_client(), req, Some(&config), DEFAULT_TIMEOUT).await;
        }

        // 429 is rate limiting, not a failure — breaker should stay closed
        assert_eq!(resolve_breaker(&config, TEST_BUCKET).state(), State::Closed);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn concurrent_requests_breaker_trips_midway() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(503).set_delay(Duration::from_millis(50)))
            .mount(&server)
            .await;

        let config = ResilienceConfig {
            retry_max: 0,
            ..test_config()
        };
        let config = Arc::new(config);

        let mut handles = Vec::new();
        for i in 0..10 {
            let client = test_client();
            let uri = server.uri();
            let cfg = Arc::clone(&config);
            handles.push(tokio::spawn(async move {
                tokio::time::sleep(Duration::from_millis(i * 30)).await;
                let req = get_request_with_headers(&uri, None, Some(TEST_BUCKET));
                send(&client, req, Some(&*cfg), DEFAULT_TIMEOUT).await
            }));
        }

        let mut ok_count = 0u32;
        let mut err_count = 0u32;
        for handle in handles {
            match handle.await.unwrap() {
                Ok(_) => ok_count += 1,
                Err(_) => err_count += 1,
            }
        }

        assert!(ok_count > 0, "some requests should have reached the server");
        assert!(err_count > 0, "breaker should have rejected some requests");
        assert_eq!(resolve_breaker(&config, TEST_BUCKET).state(), State::Open);
    }

    #[tokio::test]
    async fn send_options_retried() {
        let server = MockServer::start().await;
        Mock::given(method("OPTIONS"))
            .respond_with(ResponseTemplate::new(503))
            .up_to_n_times(1)
            .mount(&server)
            .await;
        Mock::given(method("OPTIONS"))
            .respond_with(ResponseTemplate::new(200))
            .mount(&server)
            .await;

        let config = test_config();
        let req = Request::options(server.uri())
            .body(
                Empty::new()
                    .map_err(|e| ErrorCode::InternalError(Some(e.to_string())))
                    .boxed_unsync(),
            )
            .unwrap();
        let result = send(&test_client(), req, Some(&config), DEFAULT_TIMEOUT).await;

        let resp = result.unwrap();
        assert_eq!(resp.status(), 200);

        let received = server.received_requests().await.unwrap();
        assert_eq!(received.len(), 2, "OPTIONS should have been retried once");
    }

    #[tokio::test]
    async fn direct_send_strips_headers() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200))
            .expect(1)
            .mount(&server)
            .await;

        let req = get_request_with_headers(&server.uri(), Some(5000), Some("monitoring"));
        let result = send(&test_client(), req, None, DEFAULT_TIMEOUT).await;

        result.unwrap();

        let received = server.received_requests().await.unwrap();
        assert_eq!(received.len(), 1);
        assert!(received[0].headers.get("x-omnia-timeout-ms").is_none());
        assert!(received[0].headers.get("x-omnia-upstream").is_none());
    }

    #[tokio::test]
    async fn direct_send_applies_header_timeout() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_delay(Duration::from_secs(2)))
            .mount(&server)
            .await;

        let req = get_request_with_headers(&server.uri(), Some(500), None);
        let result = send(&test_client(), req, None, DEFAULT_TIMEOUT).await;

        result.unwrap_err();
    }

    #[tokio::test]
    async fn direct_send_no_retry() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(503))
            .expect(1)
            .mount(&server)
            .await;

        let req = get_request(&server.uri());
        let result = send(&test_client(), req, None, DEFAULT_TIMEOUT).await;

        let resp = result.unwrap();
        assert_eq!(resp.status(), 503);

        let received = server.received_requests().await.unwrap();
        assert_eq!(received.len(), 1, "direct send should not retry");
    }

    #[tokio::test]
    async fn direct_send_infinite_timeout_when_zero() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_delay(Duration::from_millis(200)))
            .expect(1)
            .mount(&server)
            .await;

        let req = get_request(&server.uri());
        let result = send(&test_client(), req, None, Duration::ZERO).await;

        result.unwrap();
    }

    #[tokio::test]
    async fn timeout_faults_breaker() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_delay(Duration::from_secs(2)))
            .mount(&server)
            .await;

        let config = ResilienceConfig {
            retry_max: 0,
            ..test_config()
        };

        for _ in 0..3 {
            let req = get_request_with_headers(&server.uri(), Some(100), Some(TEST_BUCKET));
            let _ = send(&test_client(), req, Some(&config), DEFAULT_TIMEOUT).await;
        }

        assert_eq!(resolve_breaker(&config, TEST_BUCKET).state(), State::Open);
    }
}
