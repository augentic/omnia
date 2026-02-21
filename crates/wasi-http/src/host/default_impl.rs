use std::fmt::Display;

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
use wasmtime_wasi::TrappableError;
use wasmtime_wasi_http::p3::bindings::http::types::ErrorCode;
use wasmtime_wasi_http::p3::{self, RequestOptions};

pub type HttpResult<T> = Result<T, HttpError>;
pub type HttpError = TrappableError<ErrorCode>;
pub type FutureResult<T> = Box<dyn Future<Output = Result<T, ErrorCode>> + Send>;

/// Set of headers that are forbidden by by `wasmtime-wasi-http`.
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
}

impl omnia::FromEnv for ConnectOptions {
    fn from_env() -> Result<Self> {
        Self::from_env().finalize().context("issue loading connection options")
    }
}

/// Default implementation for `wasi:http`.
#[derive(Debug, Clone)]
pub struct HttpDefault;

impl Backend for HttpDefault {
    type ConnectOptions = ConnectOptions;

    #[instrument]
    async fn connect_with(options: Self::ConnectOptions) -> Result<Self> {
        Ok(Self)
    }
}

impl p3::WasiHttpCtx for HttpDefault {
    fn send_request(
        &mut self, request: Request<UnsyncBoxBody<Bytes, ErrorCode>>,
        _options: Option<RequestOptions>, fut: FutureResult<()>,
    ) -> Box<
        dyn Future<
                Output = HttpResult<(Response<UnsyncBoxBody<Bytes, ErrorCode>>, FutureResult<()>)>,
            > + Send,
    > {
        Box::new(async move {
            let (mut parts, body) = request.into_parts();

            // dedupe "Host" headers (keep the one added by wasmtime/wasip3?)
            let values = parts.headers.get_all(HOST).iter().cloned().collect::<Vec<_>>();
            if values.len() > 1 {
                parts.headers.remove(HOST);
                for v in values.into_iter().skip(1) {
                    parts.headers.append(HOST, v);
                }
            }

            // build client
            let mut builder = reqwest::Client::builder();

            // check for "Client-Cert" header
            if let Some(encoded_cert) = parts.headers.remove("Client-Cert") {
                tracing::debug!("using client certificate");
                let encoded = encoded_cert.to_str().map_err(internal_err)?;
                let bytes = Base64::decode_vec(encoded).map_err(internal_err)?;
                let identity = reqwest::Identity::from_pem(&bytes).map_err(internal_err)?;
                builder = builder.identity(identity);
            }

            // disable system proxy in tests to avoid macOS issues
            #[cfg(test)]
            let builder = builder.no_proxy();
            let client = builder.build().map_err(reqwest_err)?;

            let collected = body.collect().await.map_err(internal_err)?;

            // make request
            let resp = client
                .request(parts.method, parts.uri.to_string())
                .headers(parts.headers)
                .body(collected.to_bytes())
                .send()
                .await
                .map_err(reqwest_err)?;

            // process response
            let converted: Response<reqwest::Body> = resp.into();
            let (parts, body) = converted.into_parts();
            let body = body.map_err(reqwest_err).boxed_unsync();
            let mut response = Response::from_parts(parts, body);

            // remove forbidden headers (disallowed by `wasmtime-wasi-http`)
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

#[allow(clippy::needless_pass_by_value)]
fn reqwest_err(e: reqwest::Error) -> ErrorCode {
    if e.is_timeout() {
        ErrorCode::ConnectionTimeout
    } else if e.is_connect() {
        ErrorCode::ConnectionRefused
    } else if e.is_request() {
        ErrorCode::HttpRequestUriInvalid
    } else {
        internal_err(e)
    }
}

#[cfg(test)]
mod tests {
    use std::pin::Pin;

    use http::header::{AUTHORIZATION, CONTENT_TYPE};
    use http::{Method, StatusCode};
    use http_body_util::{Empty, Full};
    use p3::WasiHttpCtx;
    use wiremock::matchers::{body_string, header, method};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;

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

        let result = HttpDefault.handle(request).await;
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
        assert_eq!(requests[0].headers.get(HOST).unwrap().to_str().unwrap(), "localhost-2");
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

        let result = HttpDefault.handle(request).await;
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

        let result = HttpDefault.handle(request).await;
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

        let result = HttpDefault.handle(request).await;
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

        let result = HttpDefault.handle(request).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn connection_refused() {
        let request = Request::get("http://localhost:59999/test")
            .body(Empty::new().map_err(internal_err).boxed_unsync())
            .unwrap();

        let result = HttpDefault.handle(request).await;
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

        let result = HttpDefault.handle(request).await;
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

        let result = HttpDefault.handle(request).await;
        assert!(result.is_err());
    }

    // Mock `wasip3::proxy::wasi::http::handler::handle` method
    impl HttpDefault {
        async fn handle(
            &mut self, request: Request<UnsyncBoxBody<Bytes, ErrorCode>>,
        ) -> HttpResult<(Response<UnsyncBoxBody<Bytes, ErrorCode>>, FutureResult<()>)> {
            let boxed = self.send_request(request, None, Box::new(async { Ok(()) }));
            Pin::from(boxed).await
        }
    }
}
