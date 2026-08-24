use std::fmt::Display;
use std::time::Duration;

use anyhow::{Context, Result};
use base64ct::{Base64, Encoding};
use bytes::Bytes;
use fromenv::FromEnv;
use futures::{Future, TryStreamExt};
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
    #[env(from = "HTTP_CONNECT_TIMEOUT", default = "10")]
    pub connect_timeout: u64,
}

impl omnia::FromEnv for ConnectOptions {
    fn load_env() -> Result<Self> {
        // `Self::from_env()` is the builder-returning inherent the `FromEnv`
        // derive emits.
        Self::from_env().finalize().context("issue loading connection options")
    }
}

/// Reqwest-based HTTP hooks for outbound `wasi:http` requests.
#[derive(Debug, Clone)]
struct HttpHooks {
    client: reqwest::Client,
    connect_timeout: Duration,
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

// reqwest is built with `rustls-no-provider` (keeping `aws-lc-sys` out of the
// tree), which requires a process-level crypto provider before a client is
// built. Ring is the provider wasmtime-wasi-http already links; an embedder
// that installed its own provider first wins, and losing an install race
// still leaves exactly one default in place.
fn ensure_crypto_provider() {
    if rustls::crypto::CryptoProvider::get_default().is_none() {
        let _ = rustls::crypto::ring::default_provider().install_default();
    }
}

impl Backend for HttpDefault {
    type ConnectOptions = ConnectOptions;

    #[instrument]
    async fn connect_with(options: Self::ConnectOptions) -> Result<Self> {
        ensure_crypto_provider();
        let connect_timeout = Duration::from_secs(options.connect_timeout);
        let builder = reqwest::Client::builder().connect_timeout(connect_timeout);
        let client = builder.build().context("building HTTP client")?;
        Ok(Self {
            hooks: HttpHooks {
                client,
                connect_timeout,
            },
            ctx: WasiHttpCtx::default(),
        })
    }
}

impl p3::WasiHttpHooks for HttpHooks {
    fn send_request(
        &mut self, request: Request<UnsyncBoxBody<Bytes, ErrorCode>>,
        options: Option<RequestOptions>, fut: FutureResult<()>,
    ) -> Box<
        dyn Future<
                Output = HttpResult<(Response<UnsyncBoxBody<Bytes, ErrorCode>>, FutureResult<()>)>,
            > + Send,
    > {
        let shared_client = self.client.clone();
        let connect_timeout = self.connect_timeout;

        // guest-supplied timeouts from `wasi:http/types.request-options`
        let opt_connect = options.and_then(|o| o.connect_timeout);
        let opt_first_byte = options.and_then(|o| o.first_byte_timeout);
        let opt_between = options.and_then(|o| o.between_bytes_timeout);

        Box::new(async move {
            let (mut parts, body) = request.into_parts();

            // remove "Host" headers (`reqwest` adds its own)
            parts.headers.remove(HOST);

            // A one-off client is required for a client certificate or whenever the
            // guest overrides the connect/between-bytes timeouts (both are
            // client-level in `reqwest`); otherwise reuse the shared client so
            // connection pooling still applies on the common path.
            let cert = parts.headers.remove("Client-Cert");
            let client = if cert.is_some() || opt_connect.is_some() || opt_between.is_some() {
                let builder = reqwest::Client::builder()
                    .connect_timeout(opt_connect.unwrap_or(connect_timeout));
                let builder = match opt_between {
                    Some(between) => builder.read_timeout(between),
                    None => builder,
                };
                let builder = match cert {
                    Some(encoded_cert) => {
                        tracing::debug!("using client certificate");
                        let encoded = encoded_cert.to_str().map_err(internal_err)?;
                        let bytes = Base64::decode_vec(encoded).map_err(internal_err)?;
                        let identity = reqwest::Identity::from_pem(&bytes).map_err(internal_err)?;
                        builder.identity(identity)
                    }
                    None => builder,
                };
                builder.build().map_err(reqwest_err)?
            } else {
                shared_client
            };

            // Stream the outbound body instead of buffering it: a large or
            // long-lived guest body would otherwise sit in host memory in full
            // before the request even starts.
            let body = reqwest::Body::wrap_stream(
                body.into_data_stream().map_err(|e| std::io::Error::other(e.to_string())),
            );

            // make request
            let url = parts.uri.to_string();
            let send = client.request(parts.method, &url).headers(parts.headers).body(body).send();

            // Bound time-to-response (connect + first byte). The response body is
            // streamed downstream, so it is *not* part of this deadline; its
            // pacing is governed by `between_bytes` (the read timeout above).
            let resp = match opt_first_byte {
                Some(first_byte) => {
                    let budget = opt_connect.unwrap_or(connect_timeout).saturating_add(first_byte);
                    match tokio::time::timeout(budget, send).await {
                        Ok(result) => result.map_err(reqwest_err)?,
                        Err(_elapsed) => return Err(ErrorCode::ConnectionTimeout.into()),
                    }
                }
                None => send.await.map_err(reqwest_err)?,
            };

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

// Outbound behavior (body forwarding, header handling, error mapping, client
// certificates) is covered at the guest–host seam by the outbound scenarios
// in `omnia-seam-suite`'s `tests/conformance.rs`.
