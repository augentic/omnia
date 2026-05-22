use std::fmt::Display;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use base64ct::{Base64, Encoding};
use bytes::Bytes;
use fromenv::FromEnv;
use futures::Future;
use http::header::{
    CONNECTION, HOST, HeaderName, PROXY_AUTHENTICATE, PROXY_AUTHORIZATION, TRANSFER_ENCODING,
    UPGRADE,
};
use http::{Request, Response};
use http_body_util::BodyExt;
use http_body_util::combinators::UnsyncBoxBody;
use omnia::Backend;
use tracing::instrument;
use wasmtime::component::ResourceTable;
use wasmtime_wasi::TrappableError;
use wasmtime_wasi_http::WasiHttpCtx;
use wasmtime_wasi_http::p3::bindings::http::types::ErrorCode;
use wasmtime_wasi_http::p3::{self, RequestOptions, WasiHttpCtxView};

use super::circuit_breaker::{BreakerConfig, BucketRegistry};
use super::outbound::{self, ResilienceConfig};
use super::retry::RetryPolicy;

pub type HttpResult<T> = Result<T, HttpError>;
pub type HttpError = TrappableError<ErrorCode>;
pub type FutureResult<T> = Box<dyn Future<Output = Result<T, ErrorCode>> + Send>;

/// Set of headers that are forbidden by `wasmtime-wasi-http`.
pub const FORBIDDEN_HEADERS: [HeaderName; 9] = [
    CONNECTION,
    HOST,
    PROXY_AUTHENTICATE,
    PROXY_AUTHORIZATION,
    TRANSFER_ENCODING,
    UPGRADE,
    HeaderName::from_static("keep-alive"),
    HeaderName::from_static("proxy-connection"),
    HeaderName::from_static("http2-settings"),
];

#[derive(Debug, Clone, FromEnv)]
pub struct ConnectOptions {
    #[env(from = "HTTP_ADDR", default = "http://localhost:8080")]
    pub addr: String,
    #[env(from = "HTTP_CONNECT_TIMEOUT_SECS", default = "10")]
    pub connect_timeout_secs: u64,
    #[env(from = "HTTP_OUTBOUND_RESILIENCE", default = "false")]
    pub outbound_resilience: bool,
    #[env(from = "HTTP_RESPONSE_TIMEOUT_MS", default = "0")]
    pub response_timeout_ms: u64,
    #[env(from = "HTTP_RETRY_MAX", default = "2")]
    pub retry_max: u8,
    #[env(from = "HTTP_RETRY_BASE_DELAY_MS", default = "100")]
    pub retry_base_delay_ms: u64,
    #[env(from = "HTTP_RETRY_CAP_DELAY_MS", default = "1000")]
    pub retry_cap_delay_ms: u64,
    #[env(from = "HTTP_CB_SWITCH_ON_THRESHOLD", default = "10")]
    pub cb_switch_on_threshold: u32,
    #[env(from = "HTTP_CB_SWITCH_OFF_THRESHOLD", default = "5")]
    pub cb_switch_off_threshold: u32,
    #[env(from = "HTTP_CB_RESET_PERIOD_MS", default = "10000")]
    pub cb_reset_period_ms: u64,
    #[env(from = "HTTP_CB_FAULT_WINDOW_MS", default = "30000")]
    pub cb_fault_window_ms: u64,
    #[env(from = "HTTP_CB_BUCKETS", default = "")]
    pub cb_buckets: String,
}

impl omnia::FromEnv for ConnectOptions {
    fn from_env() -> Result<Self> {
        Self::from_env().finalize().context("issue loading connection options")
    }
}

/// Reqwest-based HTTP hooks for outbound `wasi:http` requests.
#[derive(Debug, Clone)]
struct HttpHooks {
    client: reqwest::Client,
    connect_timeout: Duration,
    default_timeout: Duration,
    resilience: Option<ResilienceConfig>,
}

/// Default implementation for `wasi:http`.
#[derive(Debug, Clone)]
pub struct HttpDefault {
    hooks: HttpHooks,
    ctx: WasiHttpCtx,
}

impl HttpDefault {
    /// Produce a [`WasiHttpCtxView`] by splitting borrows on inner fields.
    pub fn as_view<'a>(&'a mut self, table: &'a mut ResourceTable) -> WasiHttpCtxView<'a> {
        WasiHttpCtxView {
            hooks: &mut self.hooks,
            ctx: &mut self.ctx,
            table,
        }
    }
}

impl Backend for HttpDefault {
    type ConnectOptions = ConnectOptions;

    #[instrument]
    async fn connect_with(options: Self::ConnectOptions) -> Result<Self> {
        let connect_timeout = Duration::from_secs(options.connect_timeout_secs);
        let default_timeout = Duration::from_millis(options.response_timeout_ms);

        let builder = reqwest::Client::builder().connect_timeout(connect_timeout);

        #[cfg(test)]
        let builder = builder.no_proxy();

        let client = builder.build().context("building HTTP client")?;

        let resilience = options.outbound_resilience.then(|| {
            let breaker_config = BreakerConfig {
                switch_on_threshold: options.cb_switch_on_threshold,
                switch_off_threshold: options.cb_switch_off_threshold,
                reset_period: Duration::from_millis(options.cb_reset_period_ms),
                fault_window: Duration::from_millis(options.cb_fault_window_ms),
            };

            ResilienceConfig {
                retry_max: options.retry_max,
                retry_policy: RetryPolicy {
                    base_delay_ms: options.retry_base_delay_ms,
                    cap_delay_ms: options.retry_cap_delay_ms,
                },
                registry: Arc::new(BucketRegistry::new(&options.cb_buckets, &breaker_config)),
            }
        });

        Ok(Self {
            hooks: HttpHooks {
                client,
                connect_timeout,
                default_timeout,
                resilience,
            },
            ctx: WasiHttpCtx::default(),
        })
    }
}

impl p3::WasiHttpHooks for HttpHooks {
    fn send_request(
        &mut self, request: Request<UnsyncBoxBody<Bytes, ErrorCode>>,
        _options: Option<RequestOptions>, fut: FutureResult<()>,
    ) -> Box<
        dyn Future<
                Output = HttpResult<(Response<UnsyncBoxBody<Bytes, ErrorCode>>, FutureResult<()>)>,
            > + Send,
    > {
        let shared_client = self.client.clone();
        let connect_timeout = self.connect_timeout;
        let default_timeout = self.default_timeout;
        let resilience = self.resilience.clone();

        Box::new(async move {
            let (mut parts, body) = request.into_parts();

            // Use a one-off client when a client certificate is required, otherwise
            // reuse the shared client for connection pooling.
            let client = if let Some(encoded_cert) = parts.headers.remove("Client-Cert") {
                tracing::debug!("using client certificate");
                let encoded = encoded_cert.to_str().map_err(internal_err)?;
                let bytes = Base64::decode_vec(encoded).map_err(internal_err)?;
                let identity = reqwest::Identity::from_pem(&bytes).map_err(internal_err)?;
                let builder =
                    reqwest::Client::builder().connect_timeout(connect_timeout).identity(identity);

                #[cfg(test)]
                let builder = builder.no_proxy();

                builder.build().map_err(outbound::reqwest_to_error_code)?
            } else {
                shared_client
            };

            let request = Request::from_parts(parts, body);
            let resp = outbound::send(&client, request, resilience.as_ref(), default_timeout)
                .await
                .map_err(HttpError::from)?;

            // Process response
            let converted: Response<reqwest::Body> = resp.into();
            let (parts, body) = converted.into_parts();
            let body = body.map_err(outbound::reqwest_to_error_code).boxed_unsync();
            let mut response = Response::from_parts(parts, body);

            // Remove forbidden headers (disallowed by `wasmtime-wasi-http`)
            let headers = response.headers_mut();
            for header in &FORBIDDEN_HEADERS {
                headers.remove(header);
            }

            Ok((response, fut))
        })
    }
}

fn internal_err(e: impl Display) -> ErrorCode {
    ErrorCode::InternalError(Some(e.to_string()))
}

#[cfg(test)]
mod tests {
    use std::pin::Pin;

    use http::header::{AUTHORIZATION, CONTENT_TYPE};
    use http::{Method, StatusCode};
    use http_body_util::{Empty, Full};
    use p3::WasiHttpHooks;
    use wiremock::matchers::{body_string, header, method};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;

    async fn test_client() -> HttpDefault {
        let options = ConnectOptions {
            addr: String::new(),
            connect_timeout_secs: 10,
            outbound_resilience: false,
            response_timeout_ms: 0,
            retry_max: 2,
            retry_base_delay_ms: 100,
            retry_cap_delay_ms: 1000,
            cb_switch_on_threshold: 10,
            cb_switch_off_threshold: 5,
            cb_reset_period_ms: 10_000,
            cb_fault_window_ms: 30_000,
            cb_buckets: String::new(),
        };
        HttpDefault::connect_with(options).await.unwrap()
    }

    #[tokio::test]
    async fn multiple_host_headers() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_body_string("Hello, World!"))
            .mount(&server)
            .await;

        let request = Request::get(server.uri())
            .header(HOST, "localhost-1")
            .header(HOST, "localhost-2")
            .body(Empty::new().map_err(internal_err).boxed_unsync())
            .unwrap();

        let result = test_client().await.handle(request).await;
        assert!(result.is_ok());

        // check response
        let (response, _) = result.unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        // check body
        let body = response.into_body().collect().await.unwrap().to_bytes();
        assert_eq!(body, Bytes::from("Hello, World!"));

        // check received request
        let requests = server.received_requests().await.expect("should have requests");
        assert_eq!(requests.len(), 1);

        assert_eq!(requests[0].headers.get_all(HOST).iter().count(), 1);
        assert!(requests[0].headers.get(HOST).unwrap().to_str().unwrap().starts_with("127.0.0.1:"));
    }

    #[tokio::test]
    async fn post_with_body() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(body_string("test body"))
            .respond_with(ResponseTemplate::new(201).set_body_string("Created"))
            .mount(&server)
            .await;

        let request = Request::post(server.uri())
            .body(Full::new(Bytes::from("test body")).map_err(internal_err).boxed_unsync())
            .unwrap();

        let result = test_client().await.handle(request).await;
        assert!(result.is_ok());

        let (response, _) = result.unwrap();
        assert_eq!(response.status(), StatusCode::CREATED);
    }

    #[tokio::test]
    async fn custom_headers() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(header("X-Custom-Header", "custom-value"))
            .and(header(AUTHORIZATION, "Bearer token123"))
            .respond_with(ResponseTemplate::new(200))
            .mount(&server)
            .await;

        let request = Request::get(server.uri())
            .header("X-Custom-Header", "custom-value")
            .header(AUTHORIZATION, "Bearer token123")
            .body(Empty::new().map_err(internal_err).boxed_unsync())
            .unwrap();

        let result = test_client().await.handle(request).await;
        assert!(result.is_ok());

        let (response, _) = result.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn permitted_headers() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header(CONNECTION, "keep-alive")
                    .insert_header(TRANSFER_ENCODING, "chunked")
                    .insert_header(UPGRADE, "websocket")
                    .insert_header(CONTENT_TYPE, "application/json")
                    .insert_header("X-Safe-Header", "safe-value"),
            )
            .mount(&server)
            .await;

        let request = Request::get(server.uri())
            .body(Empty::new().map_err(internal_err).boxed_unsync())
            .unwrap();

        let result = test_client().await.handle(request).await;
        assert!(result.is_ok());

        // check response
        let (response, _) = result.unwrap();
        let headers = response.headers();

        // permitted headers are preserved
        assert_eq!(headers.get(CONTENT_TYPE).unwrap().to_str().unwrap(), "application/json");
        assert_eq!(headers.get("X-Safe-Header").unwrap().to_str().unwrap(), "safe-value");

        // verify forbidden headers are removed
        assert!(!headers.contains_key(CONNECTION));
        assert!(!headers.contains_key(TRANSFER_ENCODING));
        assert!(!headers.contains_key(UPGRADE));
    }

    #[tokio::test]
    async fn invalid_uri() {
        let body = Full::new(Bytes::from("")).map_err(internal_err).boxed_unsync();
        let request =
            Request::builder().method(Method::GET).uri("not-a-valid-uri").body(body).unwrap();

        let result = test_client().await.handle(request).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn connection_refused() {
        let request = Request::get("http://localhost:59999/test")
            .body(Empty::new().map_err(internal_err).boxed_unsync())
            .unwrap();

        let result = test_client().await.handle(request).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn client_cert_base64() {
        let server = MockServer::start().await;
        Mock::given(method("GET")).respond_with(ResponseTemplate::new(200)).mount(&server).await;

        let request = Request::get(server.uri())
            .header("Client-Cert", "not-valid-base64!!!")
            .body(Empty::new().map_err(internal_err).boxed_unsync())
            .unwrap();

        let result = test_client().await.handle(request).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn client_cert_pem() {
        let server = MockServer::start().await;
        Mock::given(method("GET")).respond_with(ResponseTemplate::new(200)).mount(&server).await;

        let invalid_pem = "invalid pem content";
        let encoded = Base64::encode_string(invalid_pem.as_bytes());
        let request = Request::get(server.uri())
            .header("Client-Cert", encoded)
            .body(Empty::new().map_err(internal_err).boxed_unsync())
            .unwrap();

        let result = test_client().await.handle(request).await;
        assert!(result.is_err());
    }

    impl HttpDefault {
        async fn handle(
            &mut self, request: Request<UnsyncBoxBody<Bytes, ErrorCode>>,
        ) -> HttpResult<(Response<UnsyncBoxBody<Bytes, ErrorCode>>, FutureResult<()>)> {
            let boxed = self.hooks.send_request(request, None, Box::new(async { Ok(()) }));
            Pin::from(boxed).await
        }
    }

    // --- Integration tests for resilience glue ---
    //
    // Resilience logic (retry, circuit breaker, timeout, header handling) is
    // thoroughly tested in `outbound::tests`. Tests here verify only the
    // integration surface that `default_impl` adds on top: the `WasiHttpHooks`
    // trait wiring, client-cert branching, forbidden-header stripping on the
    // response, and the resilience-disabled opt-out path.

    async fn resilience_client(timeout_ms: u64, retry_max: u8, cb_threshold: u32) -> HttpDefault {
        let options = ConnectOptions {
            addr: String::new(),
            connect_timeout_secs: 10,
            outbound_resilience: true,
            response_timeout_ms: timeout_ms,
            retry_max,
            retry_base_delay_ms: 10,
            retry_cap_delay_ms: 50,
            cb_switch_on_threshold: cb_threshold,
            cb_switch_off_threshold: 2,
            cb_reset_period_ms: 100,
            cb_fault_window_ms: 30_000,
            cb_buckets: String::new(),
        };
        HttpDefault::connect_with(options).await.unwrap()
    }

    #[tokio::test]
    async fn send_request_preserves_existing_behavior() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_body_string("preserved"))
            .mount(&server)
            .await;

        let mut client = test_client().await;
        let request = Request::get(server.uri())
            .body(Empty::new().map_err(internal_err).boxed_unsync())
            .unwrap();

        let result = client.handle(request).await;
        assert!(result.is_ok());
        let (resp, _) = result.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = resp.into_body().collect().await.unwrap().to_bytes();
        assert_eq!(body, Bytes::from("preserved"));
    }

    #[tokio::test]
    async fn send_request_client_cert_with_resilience() {
        let server = MockServer::start().await;
        Mock::given(method("GET")).respond_with(ResponseTemplate::new(200)).mount(&server).await;

        let mut client = resilience_client(5000, 2, 10).await;

        let request = Request::get(server.uri())
            .header("Client-Cert", "not-valid-base64!!!")
            .body(Empty::new().map_err(internal_err).boxed_unsync())
            .unwrap();

        let result = client.handle(request).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn send_request_resilience_disabled() {
        let server = MockServer::start().await;
        Mock::given(method("GET")).respond_with(ResponseTemplate::new(503)).mount(&server).await;

        let mut client = test_client().await;

        let request = Request::get(server.uri())
            .body(Empty::new().map_err(internal_err).boxed_unsync())
            .unwrap();

        let (resp, _fut) = client.handle(request).await.unwrap();
        assert_eq!(resp.status().as_u16(), 503);

        let received = server.received_requests().await.unwrap();
        assert_eq!(received.len(), 1, "resilience off: GET should not retry");

        for _ in 0..5 {
            let request = Request::get(server.uri())
                .body(Empty::new().map_err(internal_err).boxed_unsync())
                .unwrap();
            let _ = client.handle(request).await;
        }

        let received = server.received_requests().await.unwrap();
        assert_eq!(received.len(), 6, "all 6 requests should reach server without breaker");
    }
}
