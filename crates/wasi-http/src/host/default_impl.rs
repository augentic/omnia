// `derive(FromEnv)` generates undocumented `from_env`/`requirements` associated
// functions that would otherwise trip `missing_docs`.
#![allow(missing_docs)]

use std::fmt::Display;
use std::time::Duration;

use anyhow::{Context, Result};
use base64ct::{Base64, Encoding};
use fromenv::FromEnv;
use futures::{Future, TryStreamExt};
use http::{Request, Response};
use http_body_util::BodyExt;
use omnia_core::Backend;
use tracing::instrument;
use wasmtime::component::ResourceTable;
use wasmtime_wasi_http::{
    Error as HttpError, RequestOptions, WasiBody, WasiHttpCtx, WasiHttpCtxView, WasiHttpHooks,
};

use super::client_cert;

pub type FutureResult<T> = Box<dyn Future<Output = Result<T, HttpError>> + Send>;

/// Options for the default outbound `wasi:http` client.
#[derive(Debug, Clone, FromEnv)]
pub struct ConnectOptions {
    /// Connect timeout in seconds (`HTTP_CONNECT_TIMEOUT`, default 10).
    #[env(from = "HTTP_CONNECT_TIMEOUT", default = "10")]
    pub connect_timeout: u64,
}

impl omnia_core::FromEnv for ConnectOptions {
    fn load_env() -> Result<Self> {
        Self::from_env().finalize().context("issue loading connection options")
    }
}

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

impl omnia_core::HttpBorrow for HttpDefault {
    fn as_view<'a>(&'a mut self, table: &'a mut ResourceTable) -> WasiHttpCtxView<'a> {
        WasiHttpCtxView {
            hooks: &mut self.hooks,
            ctx: &mut self.ctx,
            table,
        }
    }
}

// reqwest is built with `rustls-no-provider` (keeping `aws-lc-sys` out of the
// tree), so a process-level provider must exist before a client is built; an
// embedder's own provider wins, and a lost install race still leaves one.
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

impl WasiHttpHooks for HttpHooks {
    // reqwest derives `Host` from the URL, and a guest can never supply one
    fn set_host_header(&mut self) -> bool {
        false
    }

    fn send_request(
        &mut self, request: Request<WasiBody>, options: Option<RequestOptions>,
        fut: FutureResult<()>,
    ) -> Box<dyn Future<Output = Result<(Response<WasiBody>, FutureResult<()>), HttpError>> + Send>
    {
        let shared_client = self.client.clone();
        let connect_timeout = self.connect_timeout;

        // guest-supplied timeouts from `wasi:http/types.request-options`
        let opt_connect = options.and_then(|o| o.connect_timeout);
        let opt_first_byte = options.and_then(|o| o.first_byte_timeout);
        let opt_between = options.and_then(|o| o.between_bytes_timeout);

        Box::new(async move {
            let (mut parts, body) = request.into_parts();

            // a one-off client only for client-level settings (certificate,
            // timeout overrides); the shared client keeps its pool otherwise
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
                        client_cert::validate_bundle(&bytes).map_err(internal_err)?;
                        let identity = reqwest::Identity::from_pem(&bytes).map_err(internal_err)?;
                        builder.identity(identity)
                    }
                    None => builder,
                };
                builder.build().map_err(reqwest_err)?
            } else {
                shared_client
            };

            // stream the outbound body rather than hold it whole in host memory
            let body = reqwest::Body::wrap_stream(
                body.into_data_stream().map_err(|e| std::io::Error::other(e.to_string())),
            );

            // make request
            let url = parts.uri.to_string();
            let send = client.request(parts.method, &url).headers(parts.headers).body(body).send();

            // bound connect plus first byte; the streamed body is paced by `between_bytes`
            let resp = match opt_first_byte {
                Some(first_byte) => {
                    let budget = opt_connect.unwrap_or(connect_timeout).saturating_add(first_byte);
                    match tokio::time::timeout(budget, send).await {
                        Ok(result) => result.map_err(reqwest_err)?,
                        Err(_elapsed) => return Err(HttpError::ConnectionTimeout),
                    }
                }
                None => send.await.map_err(reqwest_err)?,
            };

            // forbidden headers are stripped by the runtime before the guest sees them
            let converted: Response<reqwest::Body> = resp.into();
            let (parts, body) = converted.into_parts();
            let body = body.map_err(reqwest_err).boxed_unsync();
            let response = Response::from_parts(parts, body);

            Ok((response, fut))
        })
    }
}

fn internal_err(e: impl Display) -> HttpError {
    HttpError::InternalError(Some(e.to_string()))
}

#[allow(clippy::needless_pass_by_value)]
fn reqwest_err(e: reqwest::Error) -> HttpError {
    if e.is_timeout() {
        HttpError::ConnectionTimeout
    } else if e.is_connect() {
        HttpError::ConnectionRefused
    } else if e.is_request() {
        HttpError::HttpRequestUriInvalid
    } else {
        internal_err(e)
    }
}
