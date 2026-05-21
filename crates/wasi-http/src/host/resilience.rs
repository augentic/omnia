use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use http::Request;
use http::header::HOST;
use http_body_util::BodyExt;
use http_body_util::combinators::UnsyncBoxBody;
use wasmtime_wasi_http::p3::bindings::http::types::ErrorCode;

use super::circuit_breaker::BucketRegistry;
use super::retry::{RetryPolicy, retry_send};

/// Header names used to carry per-request resilience policy across the WASI boundary.
pub const HEADER_TIMEOUT_MS: &str = "x-omnia-timeout-ms";
pub const HEADER_UPSTREAM: &str = "x-omnia-upstream";

/// Resilience configuration: retry policy + circuit breaker registry.
///
/// When this is `Some` on `HttpHooks`, retry and circuit breaker are active.
/// When `None`, requests are sent directly with only timeout applied.
#[derive(Debug, Clone)]
pub struct ResilienceConfig {
    pub retry_max: u8,
    pub retry_policy: RetryPolicy,
    pub registry: Arc<BucketRegistry>,
}

/// Determines whether an outcome should count as a circuit breaker fault.
fn is_breaker_fault(result: &Result<&reqwest::Response, &reqwest::Error>) -> bool {
    match result {
        Err(e) => e.is_timeout() || e.is_connect(),
        Ok(resp) => resp.status().is_server_error() || resp.status().as_u16() == 429,
    }
}

/// Execute an outbound HTTP request with timeout, retry, and circuit breaker protection.
///
/// 1. Extract and strip `X-Omnia-*` headers
/// 2. Resolve breaker bucket
/// 3. Check circuit breaker — reject immediately if open
/// 4. Execute retry loop within timeout budget
/// 5. Record success/failure with the breaker
/// 6. Return response
pub async fn send_with_resilience(
    client: &reqwest::Client, request: Request<UnsyncBoxBody<Bytes, ErrorCode>>,
    config: &ResilienceConfig, default_timeout: Duration,
) -> Result<reqwest::Response, ErrorCode> {
    let (mut parts, body) = request.into_parts();

    // 1. Extract and strip resilience headers
    let timeout_ms = parts
        .headers
        .remove(HEADER_TIMEOUT_MS)
        .and_then(|v| v.to_str().ok().and_then(|s| s.parse::<u64>().ok()));
    let upstream =
        parts.headers.remove(HEADER_UPSTREAM).and_then(|v| v.to_str().ok().map(String::from));

    // Remove Host header (reqwest adds its own)
    parts.headers.remove(HOST);

    let timeout = match timeout_ms {
        Some(ms) => Some(Duration::from_millis(ms)),
        None if default_timeout.is_zero() => None,
        None => Some(default_timeout),
    };

    let url = parts.uri.to_string();

    // 2. Resolve breaker bucket
    let breaker = config.registry.resolve(upstream.as_deref(), &url);

    // 3. Check circuit breaker
    if breaker.check().is_err() {
        let bucket_name = upstream.as_deref().unwrap_or("default");
        return Err(ErrorCode::InternalError(Some(format!("circuit breaker open: {bucket_name}"))));
    }

    // 4. Collect body and execute with retry
    let collected =
        body.collect().await.map_err(|e| ErrorCode::InternalError(Some(e.to_string())))?;
    let body_bytes = collected.to_bytes();

    let result = retry_send(
        client,
        &parts.method,
        &url,
        parts.headers,
        body_bytes,
        config.retry_max,
        &config.retry_policy,
        timeout,
    )
    .await;

    // 5. Record with breaker
    let fault = is_breaker_fault(&result.as_ref());
    if fault {
        breaker.record_failure();
    } else {
        breaker.record_success();
    }

    // 6. Return
    result.map_err(reqwest_to_error_code)
}

#[allow(clippy::needless_pass_by_value)]
fn reqwest_to_error_code(e: reqwest::Error) -> ErrorCode {
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
            switch_on_threshold: 3,
            switch_off_threshold: 2,
            reset_period: Duration::from_millis(100),
            fault_window: Duration::from_millis(5000),
        }
    }

    fn test_config() -> ResilienceConfig {
        ResilienceConfig {
            retry_max: 2,
            retry_policy: RetryPolicy {
                base_delay_ms: 10,
                cap_delay_ms: 50,
            },
            registry: Arc::new(BucketRegistry::new("", &breaker_config())),
        }
    }

    fn test_config_with_buckets(names: &str) -> ResilienceConfig {
        ResilienceConfig {
            retry_max: 2,
            retry_policy: RetryPolicy {
                base_delay_ms: 10,
                cap_delay_ms: 50,
            },
            registry: Arc::new(BucketRegistry::new(names, &breaker_config())),
        }
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
        let result = send_with_resilience(&test_client(), req, &config, DEFAULT_TIMEOUT).await;

        result.unwrap_err();
    }

    #[tokio::test]
    async fn default_timeout_when_no_header() {
        let server = MockServer::start().await;
        Mock::given(method("GET")).respond_with(ResponseTemplate::new(200)).mount(&server).await;

        let req = get_request(&server.uri());
        let config = test_config();
        let result = send_with_resilience(&test_client(), req, &config, DEFAULT_TIMEOUT).await;

        result.unwrap();
    }

    #[tokio::test]
    async fn upstream_header_selects_bucket() {
        let server = MockServer::start().await;
        Mock::given(method("GET")).respond_with(ResponseTemplate::new(200)).mount(&server).await;

        let config = test_config_with_buckets("monitoring");
        let req = get_request_with_headers(&server.uri(), None, Some("monitoring"));
        let result = send_with_resilience(&test_client(), req, &config, DEFAULT_TIMEOUT).await;

        result.unwrap();
    }

    #[tokio::test]
    async fn path_segment_selects_bucket() {
        let server = MockServer::start().await;
        Mock::given(method("GET")).respond_with(ResponseTemplate::new(200)).mount(&server).await;

        let config = test_config_with_buckets("monitoring");
        let uri = format!("{}/monitoring/v2/upcoming", server.uri());
        let req = get_request(&uri);
        let result = send_with_resilience(&test_client(), req, &config, DEFAULT_TIMEOUT).await;

        result.unwrap();
    }

    #[tokio::test]
    async fn unknown_bucket_uses_default() {
        let server = MockServer::start().await;
        Mock::given(method("GET")).respond_with(ResponseTemplate::new(200)).mount(&server).await;

        let config = test_config_with_buckets("monitoring");
        let req = get_request_with_headers(&server.uri(), None, Some("nonexistent"));
        let result = send_with_resilience(&test_client(), req, &config, DEFAULT_TIMEOUT).await;

        result.unwrap();
    }

    #[tokio::test]
    async fn headers_stripped_before_forwarding() {
        let server = MockServer::start().await;
        Mock::given(method("GET")).respond_with(ResponseTemplate::new(200)).mount(&server).await;

        let req = get_request_with_headers(&server.uri(), Some(5000), Some("monitoring"));
        let config = test_config_with_buckets("monitoring");
        let result = send_with_resilience(&test_client(), req, &config, DEFAULT_TIMEOUT).await;

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
        let req = get_request(&server.uri());
        let result = send_with_resilience(&test_client(), req, &config, DEFAULT_TIMEOUT).await;

        result.unwrap();
        assert_eq!(config.registry.default_breaker().state(), State::Off);
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
        for _ in 0..3 {
            config.registry.default_breaker().record_failure();
        }
        assert_eq!(config.registry.default_breaker().state(), State::On);

        let req = get_request(&server.uri());
        let result = send_with_resilience(&test_client(), req, &config, DEFAULT_TIMEOUT).await;

        result.unwrap_err();
    }

    #[tokio::test]
    async fn open_breaker_error_code() {
        let server = MockServer::start().await;
        let config = test_config();
        for _ in 0..3 {
            config.registry.default_breaker().record_failure();
        }

        let req = get_request(&server.uri());
        let err =
            send_with_resilience(&test_client(), req, &config, DEFAULT_TIMEOUT).await.unwrap_err();

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
            let req = get_request(&server.uri());
            let _ = send_with_resilience(&test_client(), req, &config, DEFAULT_TIMEOUT).await;
        }

        assert_eq!(config.registry.default_breaker().state(), State::On);

        let req = get_request(&server.uri());
        let result = send_with_resilience(&test_client(), req, &config, DEFAULT_TIMEOUT).await;
        result.unwrap_err();
    }

    #[tokio::test]
    async fn breaker_recovers_via_half_on() {
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
            registry: Arc::new(BucketRegistry::new(
                "",
                &BreakerConfig {
                    switch_on_threshold: 3,
                    switch_off_threshold: 2,
                    reset_period: Duration::from_millis(50),
                    fault_window: Duration::from_millis(5000),
                },
            )),
        };

        for _ in 0..3 {
            let req = get_request(&server.uri());
            let _ = send_with_resilience(&test_client(), req, &config, DEFAULT_TIMEOUT).await;
        }
        assert_eq!(config.registry.default_breaker().state(), State::On);

        tokio::time::sleep(Duration::from_millis(100)).await;

        let req = get_request(&server.uri());
        let result = send_with_resilience(&test_client(), req, &config, DEFAULT_TIMEOUT).await;
        result.unwrap();

        let req = get_request(&server.uri());
        let result = send_with_resilience(&test_client(), req, &config, DEFAULT_TIMEOUT).await;
        result.unwrap();
        assert_eq!(config.registry.default_breaker().state(), State::Off);
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
            let _ = send_with_resilience(&test_client(), req, &config, DEFAULT_TIMEOUT).await;
        }

        let req = get_request_with_headers(&server_b.uri(), None, Some("b"));
        let result = send_with_resilience(&test_client(), req, &config, DEFAULT_TIMEOUT).await;
        result.unwrap();
    }

    #[tokio::test]
    async fn non_retryable_method_still_uses_breaker() {
        let server = MockServer::start().await;
        Mock::given(method("POST")).respond_with(ResponseTemplate::new(503)).mount(&server).await;

        let config = test_config();

        for _ in 0..3 {
            let req = post_request(&server.uri());
            let _ = send_with_resilience(&test_client(), req, &config, DEFAULT_TIMEOUT).await;
        }

        assert_eq!(config.registry.default_breaker().state(), State::On);
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
            let req = get_request(&server.uri());
            let _ = send_with_resilience(&test_client(), req, &config, DEFAULT_TIMEOUT).await;
        }

        assert_eq!(config.registry.default_breaker().state(), State::Off);
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
        let req = get_request(&server.uri());
        let result = send_with_resilience(&test_client(), req, &config, DEFAULT_TIMEOUT).await;

        result.unwrap();
        assert_eq!(config.registry.default_breaker().state(), State::Off);
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
            registry: Arc::new(BucketRegistry::new("", &breaker_config())),
        };

        let start = Instant::now();
        let req = get_request(&server.uri());
        let _ = send_with_resilience(&test_client(), req, &config, Duration::from_secs(3)).await;

        assert!(start.elapsed() < Duration::from_secs(6));
    }

    #[tokio::test]
    async fn path_segment_with_version_prefix() {
        let server = MockServer::start().await;
        Mock::given(method("GET")).respond_with(ResponseTemplate::new(200)).mount(&server).await;

        let config = test_config_with_buckets("monitoring");
        let uri = format!("{}/monitoring/v2/foo", server.uri());
        let req = get_request(&uri);
        let result = send_with_resilience(&test_client(), req, &config, DEFAULT_TIMEOUT).await;
        result.unwrap();
    }

    #[tokio::test]
    async fn four29_retried_then_faults_breaker() {
        let server = MockServer::start().await;
        Mock::given(method("GET")).respond_with(ResponseTemplate::new(429)).mount(&server).await;

        let config = test_config();

        for _ in 0..3 {
            let req = get_request(&server.uri());
            let _ = send_with_resilience(&test_client(), req, &config, DEFAULT_TIMEOUT).await;
        }

        assert_eq!(config.registry.default_breaker().state(), State::On);
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

        // Phase 1: send requests in waves — early ones trip the breaker,
        // later ones arrive after the breaker is ON and get rejected.
        let mut handles = Vec::new();
        for i in 0..10 {
            let client = test_client();
            let uri = server.uri();
            let cfg = Arc::clone(&config);
            handles.push(tokio::spawn(async move {
                // Stagger requests slightly so early failures record before later checks
                tokio::time::sleep(Duration::from_millis(i * 30)).await;
                let req = Request::get(&uri)
                    .body(
                        Empty::new()
                            .map_err(|e| ErrorCode::InternalError(Some(e.to_string())))
                            .boxed_unsync(),
                    )
                    .unwrap();
                send_with_resilience(&client, req, &cfg, DEFAULT_TIMEOUT).await
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
        assert_eq!(config.registry.default_breaker().state(), State::On);
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
        let result = send_with_resilience(&test_client(), req, &config, DEFAULT_TIMEOUT).await;

        let resp = result.unwrap();
        assert_eq!(resp.status(), 200);

        let received = server.received_requests().await.unwrap();
        assert_eq!(received.len(), 2, "OPTIONS should have been retried once");
    }
}
