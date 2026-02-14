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
use qwasr::Backend;
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

impl qwasr::FromEnv for ConnectOptions {
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
            let collected = body.collect().await.map_err(internal_error)?;

            // build reqwest::Request
            let mut client_builder = reqwest::Client::builder();

            // check for "Client-Cert" header
            if let Some(encoded_cert) = parts.headers.remove("Client-Cert") {
                tracing::debug!("using client certificate");
                let encoded = encoded_cert.to_str().map_err(internal_error)?;
                let bytes = Base64::decode_vec(encoded).map_err(internal_error)?;
                let identity = reqwest::Identity::from_pem(&bytes).map_err(internal_error)?;
                client_builder = client_builder.identity(identity);
            }

            // HACK: remove host header to appease Azure Frontdoor
            parts.headers.remove("Host");
            client_builder = client_builder.default_headers(parts.headers);

            // Disable system proxy in tests to avoid macOS system-configuration issues
            #[cfg(test)]
            let client_builder = client_builder.no_proxy();

            let client = client_builder.build().map_err(reqwest_error)?;

            // make request
            let resp = client
                .request(parts.method, parts.uri.to_string())
                .body(collected.to_bytes())
                .send()
                .await
                .map_err(reqwest_error)?;

            // process response
            let converted: Response<reqwest::Body> = resp.into();
            let (parts, body) = converted.into_parts();
            let body = body.map_err(reqwest_error).boxed_unsync();
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

fn internal_error(e: impl Display) -> ErrorCode {
    ErrorCode::InternalError(Some(e.to_string()))
}

#[allow(clippy::needless_pass_by_value)]
fn reqwest_error(e: reqwest::Error) -> ErrorCode {
    if e.is_timeout() {
        ErrorCode::ConnectionTimeout
    } else if e.is_connect() {
        ErrorCode::ConnectionRefused
    } else if e.is_request() {
        ErrorCode::HttpRequestUriInvalid
    } else {
        internal_error(e)
    }
}

#[cfg(test)]
mod tests {
    use std::pin::Pin;

    use http::{Method, StatusCode};
    use http_body_util::Full;
    use p3::WasiHttpCtx;
    use wiremock::matchers::{body_string, header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;

    #[tokio::test]
    async fn get_method() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/test"))
            .respond_with(ResponseTemplate::new(200).set_body_string("Hello, World!"))
            .mount(&server)
            .await;

        let uri = format!("{}/test", server.uri());
        let body = Full::new(Bytes::from("")).map_err(internal_error).boxed_unsync();
        let request = Request::builder().method(Method::GET).uri(&uri).body(body).unwrap();

        let result = HttpDefault.handle(request).await;

        assert!(result.is_ok());
        let (response, _) = result.unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        let body = response.into_body().collect().await.unwrap().to_bytes();
        assert_eq!(body, Bytes::from("Hello, World!"));

        let requests = server.received_requests().await.expect("should have requests");
        assert_eq!(requests.len(), 1);
        println!("requests: {:?}", requests[0].headers);
    }

    #[tokio::test]
    async fn post_with_body() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/echo"))
            .and(body_string("test body"))
            .respond_with(ResponseTemplate::new(201).set_body_string("Created"))
            .mount(&server)
            .await;

        let uri = format!("{}/echo", server.uri());
        let body = Full::new(Bytes::from("test body")).map_err(internal_error).boxed_unsync();
        let request = Request::builder().method(Method::POST).uri(&uri).body(body).unwrap();

        let result = HttpDefault.handle(request).await;

        assert!(result.is_ok());
        let (response, _) = result.unwrap();
        assert_eq!(response.status(), StatusCode::CREATED);
    }

    #[tokio::test]
    async fn custom_headers() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/headers"))
            .and(header("X-Custom-Header", "custom-value"))
            .and(header("Authorization", "Bearer token123"))
            .respond_with(ResponseTemplate::new(200))
            .mount(&server)
            .await;

        let uri = format!("{}/headers", server.uri());
        let body = Full::new(Bytes::from("")).map_err(internal_error).boxed_unsync();
        let mut request = Request::builder().method(Method::GET).uri(&uri).body(body).unwrap();
        request
            .headers_mut()
            .insert(HeaderName::from_static("x-custom-header"), "custom-value".parse().unwrap());
        request
            .headers_mut()
            .insert(http::header::AUTHORIZATION, "Bearer token123".parse().unwrap());

        let result = HttpDefault.handle(request).await;

        assert!(result.is_ok());
        let (response, _) = result.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn forbidden_headers() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/forbidden"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("Connection", "keep-alive")
                    .insert_header("Transfer-Encoding", "chunked")
                    .insert_header("Upgrade", "websocket")
                    .insert_header("X-Safe-Header", "safe-value"),
            )
            .mount(&server)
            .await;

        let uri = format!("{}/forbidden", server.uri());
        let body = Full::new(Bytes::from("")).map_err(internal_error).boxed_unsync();
        let request = Request::builder().method(Method::GET).uri(&uri).body(body).unwrap();

        let result = HttpDefault.handle(request).await;

        assert!(result.is_ok());
        let (response, _) = result.unwrap();

        // Verify forbidden headers are removed
        assert!(!response.headers().contains_key(CONNECTION));
        assert!(!response.headers().contains_key(TRANSFER_ENCODING));
        assert!(!response.headers().contains_key(UPGRADE));

        // Verify safe headers are preserved
        assert_eq!(
            response.headers().get("X-Safe-Header").unwrap().to_str().unwrap(),
            "safe-value"
        );
    }

    #[tokio::test]
    async fn invalid_uri() {
        let body = Full::new(Bytes::from("")).map_err(internal_error).boxed_unsync();
        let request =
            Request::builder().method(Method::GET).uri("not-a-valid-uri").body(body).unwrap();

        let result = HttpDefault.handle(request).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn connection_refused() {
        let uri = "http://localhost:59999/test";
        let body = Full::new(Bytes::from("")).map_err(internal_error).boxed_unsync();
        let request = Request::builder().method(Method::GET).uri(uri).body(body).unwrap();

        let result = HttpDefault.handle(request).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn invalid_client_cert_base64() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/secure"))
            .respond_with(ResponseTemplate::new(200))
            .mount(&server)
            .await;

        let uri = format!("{}/secure", server.uri());
        let body = Full::new(Bytes::from("")).map_err(internal_error).boxed_unsync();
        let mut request = Request::builder().method(Method::GET).uri(&uri).body(body).unwrap();
        request
            .headers_mut()
            .insert(HeaderName::from_static("client-cert"), "not-valid-base64!!!".parse().unwrap());

        let result = HttpDefault.handle(request).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn invalid_client_cert_pem() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/secure"))
            .respond_with(ResponseTemplate::new(200))
            .mount(&server)
            .await;

        let invalid_pem = "invalid pem content";
        let encoded = Base64::encode_string(invalid_pem.as_bytes());
        let uri = format!("{}/secure", server.uri());
        let body = Full::new(Bytes::from("")).map_err(internal_error).boxed_unsync();
        let mut request = Request::builder().method(Method::GET).uri(&uri).body(body).unwrap();
        request
            .headers_mut()
            .insert(HeaderName::from_static("client-cert"), encoded.parse().unwrap());

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
