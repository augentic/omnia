use std::time::{Duration, Instant};

use bytes::Bytes;
use http::{HeaderMap, Method};
use rand::RngExt;

/// Retry policy with jittered exponential backoff.
#[derive(Debug, Clone)]
pub struct RetryPolicy {
    pub base_delay_ms: u64,
    pub cap_delay_ms: u64,
}

impl RetryPolicy {
    fn delay_for_attempt(&self, attempt: u8) -> Duration {
        let exp = self.base_delay_ms.saturating_mul(1u64 << attempt);
        let capped = exp.min(self.cap_delay_ms);
        let jittered = rand::rng().random_range(0..=capped);
        Duration::from_millis(jittered)
    }
}

/// Whether the given HTTP method is safe to retry (idempotent / read-only).
const fn is_retryable_method(method: &Method) -> bool {
    matches!(method, &Method::GET | &Method::HEAD | &Method::OPTIONS)
}

/// Whether a reqwest response status indicates a retryable server-side issue.
const fn is_retryable_status(status: u16) -> bool {
    matches!(status, 429 | 502 | 503 | 504)
}

/// Parse a `Retry-After` header value as seconds.
///
/// Only the integer-seconds format is supported. HTTP-date values (e.g.
/// `Wed, 21 Oct 2015 07:28:00 GMT`) are silently ignored and fall back to
/// jittered exponential backoff.
fn parse_retry_after(headers: &reqwest::header::HeaderMap) -> Option<Duration> {
    let val = headers.get(reqwest::header::RETRY_AFTER)?.to_str().ok()?;
    let secs: u64 = val.trim().parse().ok()?;
    Some(Duration::from_secs(secs))
}

/// Execute an HTTP request with retries inside an optional timeout budget.
///
/// Only GET/HEAD/OPTIONS are retried. Non-retryable methods execute once.
/// `timeout = None` means no deadline (infinite budget).
/// Returns the final response or error after exhausting retries.
#[allow(clippy::too_many_arguments)]
pub async fn retry_send(
    client: &reqwest::Client, method: &Method, url: &str, headers: HeaderMap, body: Bytes,
    max_retries: u8, policy: &RetryPolicy, timeout: Option<Duration>,
) -> Result<reqwest::Response, reqwest::Error> {
    let deadline = timeout.map(|t| Instant::now() + t);
    let retryable = is_retryable_method(method);
    let effective_retries = if retryable { max_retries } else { 0 };

    let mut last_resp: Result<reqwest::Response, reqwest::Error>;

    // Always execute at least one attempt
    let mut builder =
        client.request(method.clone(), url).headers(headers.clone()).body(body.clone());
    if let Some(d) = deadline {
        builder = builder.timeout(d.saturating_duration_since(Instant::now()));
    }
    last_resp = builder.send().await;

    for attempt in 0..effective_retries {
        let should_retry = match &last_resp {
            Err(e) => e.is_timeout() || e.is_connect(),
            Ok(r) => is_retryable_status(r.status().as_u16()),
        };
        if !should_retry {
            break;
        }

        // Compute delay: prefer Retry-After for 429, else jittered exponential
        let delay = if let Ok(ref r) = last_resp
            && r.status().as_u16() == 429
        {
            parse_retry_after(r.headers()).unwrap_or_else(|| policy.delay_for_attempt(attempt))
        } else {
            policy.delay_for_attempt(attempt)
        };

        if let Some(d) = deadline {
            let budget_left = d.saturating_duration_since(Instant::now());
            if budget_left < Duration::from_millis(policy.base_delay_ms) {
                break;
            }
            tokio::time::sleep(delay.min(budget_left)).await;
        } else {
            tokio::time::sleep(delay).await;
        }

        // Check budget before next attempt
        if let Some(d) = deadline {
            let remaining = d.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                break;
            }
            let mut builder =
                client.request(method.clone(), url).headers(headers.clone()).body(body.clone());
            builder = builder.timeout(remaining);
            last_resp = builder.send().await;
        } else {
            last_resp = client
                .request(method.clone(), url)
                .headers(headers.clone())
                .body(body.clone())
                .send()
                .await;
        }
    }

    last_resp
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use wiremock::matchers::method;
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;

    fn test_client() -> reqwest::Client {
        reqwest::Client::builder().no_proxy().build().unwrap()
    }

    fn test_policy() -> RetryPolicy {
        RetryPolicy {
            base_delay_ms: 10,
            cap_delay_ms: 50,
        }
    }

    #[tokio::test]
    async fn no_retry_on_success() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200))
            .expect(1)
            .mount(&server)
            .await;

        let resp = retry_send(
            &test_client(),
            &Method::GET,
            &server.uri(),
            HeaderMap::new(),
            Bytes::new(),
            2,
            &test_policy(),
            Some(Duration::from_secs(5)),
        )
        .await
        .unwrap();

        assert_eq!(resp.status(), 200);
    }

    #[tokio::test]
    async fn success_after_retry() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(503))
            .up_to_n_times(1)
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200))
            .expect(1)
            .mount(&server)
            .await;

        let resp = retry_send(
            &test_client(),
            &Method::GET,
            &server.uri(),
            HeaderMap::new(),
            Bytes::new(),
            2,
            &test_policy(),
            Some(Duration::from_secs(5)),
        )
        .await
        .unwrap();

        assert_eq!(resp.status(), 200);
    }

    #[tokio::test]
    async fn retries_on_transient_server_errors() {
        for status in [502, 503, 504] {
            let server = MockServer::start().await;
            Mock::given(method("GET"))
                .respond_with(ResponseTemplate::new(status))
                .up_to_n_times(1)
                .mount(&server)
                .await;
            Mock::given(method("GET"))
                .respond_with(ResponseTemplate::new(200))
                .mount(&server)
                .await;

            let resp = retry_send(
                &test_client(),
                &Method::GET,
                &server.uri(),
                HeaderMap::new(),
                Bytes::new(),
                2,
                &test_policy(),
                Some(Duration::from_secs(5)),
            )
            .await
            .unwrap();

            assert_eq!(resp.status(), 200, "should recover after transient {status}");
        }
    }

    #[tokio::test]
    async fn retries_on_429_with_retry_after() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(429).insert_header("Retry-After", "1"))
            .up_to_n_times(1)
            .mount(&server)
            .await;
        Mock::given(method("GET")).respond_with(ResponseTemplate::new(200)).mount(&server).await;

        let resp = retry_send(
            &test_client(),
            &Method::GET,
            &server.uri(),
            HeaderMap::new(),
            Bytes::new(),
            2,
            &test_policy(),
            Some(Duration::from_secs(10)),
        )
        .await
        .unwrap();

        assert_eq!(resp.status(), 200);
    }

    #[tokio::test]
    async fn retries_on_429_without_retry_after() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(429))
            .up_to_n_times(1)
            .mount(&server)
            .await;
        Mock::given(method("GET")).respond_with(ResponseTemplate::new(200)).mount(&server).await;

        let resp = retry_send(
            &test_client(),
            &Method::GET,
            &server.uri(),
            HeaderMap::new(),
            Bytes::new(),
            2,
            &test_policy(),
            Some(Duration::from_secs(5)),
        )
        .await
        .unwrap();

        assert_eq!(resp.status(), 200);
    }

    #[tokio::test]
    async fn retry_after_exceeds_budget() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(429).insert_header("Retry-After", "60"))
            .mount(&server)
            .await;

        let resp = retry_send(
            &test_client(),
            &Method::GET,
            &server.uri(),
            HeaderMap::new(),
            Bytes::new(),
            2,
            &test_policy(),
            Some(Duration::from_secs(2)),
        )
        .await
        .unwrap();

        // Should return the 429 since budget doesn't allow a retry after 60s
        assert_eq!(resp.status(), 429);
    }

    #[tokio::test]
    async fn no_retry_on_get_success_codes() {
        for status in [200, 201, 204, 301, 404] {
            let server = MockServer::start().await;
            Mock::given(method("GET"))
                .respond_with(ResponseTemplate::new(status))
                .expect(1)
                .mount(&server)
                .await;

            let resp = retry_send(
                &test_client(),
                &Method::GET,
                &server.uri(),
                HeaderMap::new(),
                Bytes::new(),
                2,
                &test_policy(),
                Some(Duration::from_secs(5)),
            )
            .await
            .unwrap();

            assert_eq!(resp.status().as_u16(), status);
        }
    }

    #[tokio::test]
    async fn no_retry_on_mutating_methods() {
        for m in [Method::POST, Method::PUT, Method::PATCH, Method::DELETE] {
            let server = MockServer::start().await;
            Mock::given(method(m.as_str()))
                .respond_with(ResponseTemplate::new(503))
                .expect(1)
                .mount(&server)
                .await;

            let resp = retry_send(
                &test_client(),
                &m,
                &server.uri(),
                HeaderMap::new(),
                Bytes::new(),
                2,
                &test_policy(),
                Some(Duration::from_secs(5)),
            )
            .await
            .unwrap();

            assert_eq!(resp.status(), 503, "{m} should not be retried");
        }
    }

    #[tokio::test]
    async fn no_retry_on_non_transient_errors() {
        for status in [400, 401, 403, 404, 500] {
            let server = MockServer::start().await;
            Mock::given(method("GET"))
                .respond_with(ResponseTemplate::new(status))
                .expect(1)
                .mount(&server)
                .await;

            let resp = retry_send(
                &test_client(),
                &Method::GET,
                &server.uri(),
                HeaderMap::new(),
                Bytes::new(),
                2,
                &test_policy(),
                Some(Duration::from_secs(5)),
            )
            .await
            .unwrap();

            assert_eq!(resp.status().as_u16(), status, "{status} should not trigger retry");
        }
    }

    #[tokio::test]
    async fn exhausts_retries_returns_last_error() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(503))
            .expect(3) // 1 original + 2 retries
            .mount(&server)
            .await;

        let resp = retry_send(
            &test_client(),
            &Method::GET,
            &server.uri(),
            HeaderMap::new(),
            Bytes::new(),
            2,
            &test_policy(),
            Some(Duration::from_secs(5)),
        )
        .await
        .unwrap();

        assert_eq!(resp.status(), 503);
    }

    #[tokio::test]
    async fn max_retries_zero_means_single_attempt() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(503))
            .expect(1)
            .mount(&server)
            .await;

        let resp = retry_send(
            &test_client(),
            &Method::GET,
            &server.uri(),
            HeaderMap::new(),
            Bytes::new(),
            0,
            &test_policy(),
            Some(Duration::from_secs(5)),
        )
        .await
        .unwrap();

        assert_eq!(resp.status(), 503);
    }

    #[tokio::test]
    async fn timeout_exhausts_budget() {
        // With a shared-budget model, the per-attempt timeout = remaining budget.
        // A timeout on the first attempt consumes the budget. Verify that when
        // a timeout occurs and budget is exhausted, we return the error gracefully.
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_delay(Duration::from_secs(10)))
            .mount(&server)
            .await;

        let result = retry_send(
            &test_client(),
            &Method::GET,
            &server.uri(),
            HeaderMap::new(),
            Bytes::new(),
            2,
            &test_policy(),
            Some(Duration::from_millis(500)),
        )
        .await;

        assert!(result.is_err());
        assert!(result.unwrap_err().is_timeout());
    }

    #[tokio::test]
    async fn retries_on_connection_refused() {
        // First attempt: bad port (connection refused). Can't easily mock a recovery
        // for connection errors, so we just verify it doesn't panic and returns an error.
        let result = retry_send(
            &test_client(),
            &Method::GET,
            "http://127.0.0.1:59998/test",
            HeaderMap::new(),
            Bytes::new(),
            1,
            &test_policy(),
            Some(Duration::from_secs(3)),
        )
        .await;

        result.unwrap_err();
    }

    #[tokio::test]
    async fn respects_timeout_budget() {
        let server = MockServer::start().await;
        Mock::given(method("GET")).respond_with(ResponseTemplate::new(503)).mount(&server).await;

        let start = Instant::now();
        let resp = retry_send(
            &test_client(),
            &Method::GET,
            &server.uri(),
            HeaderMap::new(),
            Bytes::new(),
            5,
            &RetryPolicy {
                base_delay_ms: 2000,
                cap_delay_ms: 5000,
            },
            Some(Duration::from_millis(500)),
        )
        .await
        .unwrap();

        assert_eq!(resp.status(), 503);
        // Should have exited quickly rather than retrying for 5 attempts
        assert!(start.elapsed() < Duration::from_secs(3));
    }

    #[tokio::test]
    async fn per_attempt_timeout_decreases() {
        // With a very tight budget, second attempt should have less time
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(503).set_delay(Duration::from_millis(100)))
            .up_to_n_times(1)
            .mount(&server)
            .await;
        Mock::given(method("GET")).respond_with(ResponseTemplate::new(200)).mount(&server).await;

        let resp = retry_send(
            &test_client(),
            &Method::GET,
            &server.uri(),
            HeaderMap::new(),
            Bytes::new(),
            2,
            &test_policy(),
            Some(Duration::from_secs(5)),
        )
        .await
        .unwrap();

        assert_eq!(resp.status(), 200);
    }

    #[tokio::test]
    async fn backoff_bounded_by_cap() {
        let policy = RetryPolicy {
            base_delay_ms: 100,
            cap_delay_ms: 100_000,
        };

        // Deterministic: every sample must be within [0, min(base*2^attempt, cap)]
        for attempt in 0..8u8 {
            let expected_cap = (100u64.saturating_mul(1u64 << attempt)).min(100_000);
            for _ in 0..50 {
                let d = policy.delay_for_attempt(attempt);
                assert!(
                    d <= Duration::from_millis(expected_cap),
                    "attempt {attempt}: {d:?} exceeded cap {expected_cap}ms"
                );
            }
        }
    }

    #[tokio::test]
    async fn backoff_cap_saturates() {
        let policy = RetryPolicy {
            base_delay_ms: 100,
            cap_delay_ms: 500,
        };
        // At attempt 5: base*2^5 = 3200, capped to 500
        for _ in 0..50 {
            let d = policy.delay_for_attempt(5);
            assert!(d <= Duration::from_millis(500));
        }
    }

    #[tokio::test]
    async fn jitter_produces_distinct_values() {
        let policy = RetryPolicy {
            base_delay_ms: 1_000_000,
            cap_delay_ms: 1_000_000,
        };
        // Range [0, 1_000_000ms] — probability of 100 identical values ≈ 0
        let delays: Vec<_> = (0..100).map(|_| policy.delay_for_attempt(0)).collect();
        let distinct = delays.iter().collect::<std::collections::HashSet<_>>().len();
        assert!(distinct > 1, "100 samples from [0, 1_000_000ms] should not all be identical");
    }

    #[tokio::test]
    async fn body_preserved_across_retries() {
        use wiremock::matchers::body_string;

        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(body_string("hello"))
            .respond_with(ResponseTemplate::new(503))
            .up_to_n_times(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(body_string("hello"))
            .respond_with(ResponseTemplate::new(200))
            .mount(&server)
            .await;

        let resp = retry_send(
            &test_client(),
            &Method::GET,
            &server.uri(),
            HeaderMap::new(),
            Bytes::from("hello"),
            2,
            &test_policy(),
            Some(Duration::from_secs(5)),
        )
        .await
        .unwrap();

        assert_eq!(resp.status(), 200);

        let received = server.received_requests().await.unwrap();
        assert_eq!(received.len(), 2);
        assert_eq!(received[0].body, b"hello");
        assert_eq!(received[1].body, b"hello");
    }

    #[tokio::test]
    async fn remaining_budget_given_to_retry() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(503))
            .up_to_n_times(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_delay(Duration::from_secs(2)))
            .mount(&server)
            .await;

        let resp = retry_send(
            &test_client(),
            &Method::GET,
            &server.uri(),
            HeaderMap::new(),
            Bytes::new(),
            2,
            &test_policy(),
            Some(Duration::from_secs(5)),
        )
        .await
        .unwrap();

        assert_eq!(resp.status(), 200, "retry should get the remaining ~5s budget");
    }

    #[tokio::test]
    async fn retry_after_httpdate_falls_back_to_jitter() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(
                ResponseTemplate::new(429)
                    .insert_header("Retry-After", "Wed, 21 Oct 2025 07:28:00 GMT"),
            )
            .up_to_n_times(1)
            .mount(&server)
            .await;
        Mock::given(method("GET")).respond_with(ResponseTemplate::new(200)).mount(&server).await;

        let start = Instant::now();
        let resp = retry_send(
            &test_client(),
            &Method::GET,
            &server.uri(),
            HeaderMap::new(),
            Bytes::new(),
            2,
            &test_policy(),
            Some(Duration::from_secs(5)),
        )
        .await
        .unwrap();

        assert_eq!(resp.status(), 200);
        // HTTP-date is unparseable as integer — should fall back to jitter (<1s)
        assert!(start.elapsed() < Duration::from_secs(2));
    }

    #[tokio::test]
    async fn head_retried_on_503() {
        let server = MockServer::start().await;
        Mock::given(method("HEAD"))
            .respond_with(ResponseTemplate::new(503))
            .up_to_n_times(1)
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("HEAD"))
            .respond_with(ResponseTemplate::new(200))
            .expect(1)
            .mount(&server)
            .await;

        let resp = retry_send(
            &test_client(),
            &Method::HEAD,
            &server.uri(),
            HeaderMap::new(),
            Bytes::new(),
            2,
            &test_policy(),
            Some(Duration::from_secs(5)),
        )
        .await
        .unwrap();

        assert_eq!(resp.status(), 200);
    }

    #[tokio::test]
    async fn no_timeout_budget_retries_fully() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(503))
            .up_to_n_times(2)
            .mount(&server)
            .await;
        Mock::given(method("GET")).respond_with(ResponseTemplate::new(200)).mount(&server).await;

        let resp = retry_send(
            &test_client(),
            &Method::GET,
            &server.uri(),
            HeaderMap::new(),
            Bytes::new(),
            3,
            &test_policy(),
            None,
        )
        .await
        .unwrap();

        assert_eq!(resp.status(), 200);

        let received = server.received_requests().await.unwrap();
        assert_eq!(received.len(), 3, "should have retried twice with no budget limit");
    }
}
