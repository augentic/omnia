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

            // check for client certificate in headers
            if let Some(encoded_cert) = parts.headers.remove("Client-Cert") {
                tracing::debug!("using client certificate");
                let encoded = encoded_cert.to_str().map_err(internal_error)?;
                let bytes = Base64::decode_vec(encoded).map_err(internal_error)?;
                let identity = reqwest::Identity::from_pem(&bytes).map_err(internal_error)?;
                client_builder = client_builder.identity(identity);
            }

            // remove Host header (it is added by reqwest automatically)
            parts.headers.remove("Host");

            // make request
            let client = client_builder.build().map_err(reqwest_error)?;
            let resp = client
                .request(parts.method, parts.uri.to_string())
                .headers(parts.headers)
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
    // } else if e.is_body() {
    //     ErrorCode::HttpRequestBodyRead
    } else {
        internal_error(e)
    }
}
