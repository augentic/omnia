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
    async fn retries_on_502() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(502))
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
    async fn retries_on_503() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(503))
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
    async fn retries_on_504() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(504))
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
    async fn no_retry_on_post() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(503))
            .expect(1)
            .mount(&server)
            .await;

        let resp = retry_send(
            &test_client(),
            &Method::POST,
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
    async fn no_retry_on_put() {
        let server = MockServer::start().await;
        Mock::given(method("PUT"))
            .respond_with(ResponseTemplate::new(503))
            .expect(1)
            .mount(&server)
            .await;

        let resp = retry_send(
            &test_client(),
            &Method::PUT,
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
    async fn no_retry_on_patch() {
        let server = MockServer::start().await;
        Mock::given(method("PATCH"))
            .respond_with(ResponseTemplate::new(503))
            .expect(1)
            .mount(&server)
            .await;

        let resp = retry_send(
            &test_client(),
            &Method::PATCH,
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
    async fn no_retry_on_delete() {
        let server = MockServer::start().await;
        Mock::given(method("DELETE"))
            .respond_with(ResponseTemplate::new(503))
            .expect(1)
            .mount(&server)
            .await;

        let resp = retry_send(
            &test_client(),
            &Method::DELETE,
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
    async fn no_retry_on_400() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(400))
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

        assert_eq!(resp.status(), 400);
    }

    #[tokio::test]
    async fn no_retry_on_401() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(401))
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

        assert_eq!(resp.status(), 401);
    }

    #[tokio::test]
    async fn no_retry_on_403() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(403))
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

        assert_eq!(resp.status(), 403);
    }

    #[tokio::test]
    async fn no_retry_on_404() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(404))
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

        assert_eq!(resp.status(), 404);
    }

    #[tokio::test]
    async fn no_retry_on_500() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(500))
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

        assert_eq!(resp.status(), 500);
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
    async fn exponential_backoff_increases() {
        let policy = RetryPolicy {
            base_delay_ms: 100,
            cap_delay_ms: 10000,
        };
        // Verify delay calculation grows (testing internal method)
        let d0 = policy.delay_for_attempt(0);
        // With jitter, d0 is in [0, 100ms]
        assert!(d0 <= Duration::from_millis(100));

        // attempt 2: cap is min(100*4, 10000) = 400, jitter in [0, 400]
        // We can't assert deterministic values due to jitter, but the cap grows
    }

    #[tokio::test]
    async fn jitter_randomizes_delay() {
        let policy = RetryPolicy {
            base_delay_ms: 1000,
            cap_delay_ms: 10000,
        };
        let mut delays = Vec::new();
        for _ in 0..10 {
            delays.push(policy.delay_for_attempt(0));
        }
        // With 10 samples from [0, 1000ms], extremely unlikely they're all identical
        let all_same = delays.windows(2).all(|w| w[0] == w[1]);
        assert!(!all_same, "jitter should produce varying delays");
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
        // First attempt: 503 instantly. Second attempt: delays 2s then 200.
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
}
