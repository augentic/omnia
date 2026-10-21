use std::error::Error;
use std::fmt::Display;
use std::io;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{self, Poll, ready};
use std::time::Duration;

use anyhow::{Context, Result};
use base64ct::{Base64, Encoding};
use bytes::Bytes;
use futures::{Future, TryStreamExt};
use http::{Request, Response};
use http_body::{Body, Frame, SizeHint};
use http_body_util::BodyExt;
use omnia_core::Backend;
use tokio::time::Sleep;
use tracing::instrument;
use wasmtime::component::ResourceTable;
use wasmtime_wasi_http::{
    Error as HttpError, RequestOptions, WasiBody, WasiHttpCtx, WasiHttpCtxView, WasiHttpHooks,
};

use super::client_cert;

pub type FutureResult<T> = Box<dyn Future<Output = Result<T, HttpError>> + Send>;

#[expect(missing_docs, reason = "`FromEnv` has no docs")]
mod config {
    use fromenv::FromEnv;

    /// Options for the default outbound `wasi:http` client.
    #[derive(Debug, Clone, FromEnv)]
    pub struct ConnectOptions {
        /// Connect timeout in seconds (`HTTP_CONNECT_TIMEOUT`, default 10).
        #[env(from = "HTTP_CONNECT_TIMEOUT", default = "10")]
        pub connect_timeout: u64,
    }
}
pub use config::ConnectOptions;

impl omnia_core::FromEnv for ConnectOptions {
    fn load_env() -> Result<Self> {
        Self::from_env().finalize().context("issue loading connection options")
    }
}

#[derive(Debug, Clone)]
struct HttpHooks {
    clients: Clients,
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
        let clients = Clients::new(Duration::from_secs(options.connect_timeout))
            .context("building HTTP client")?;
        Ok(Self {
            hooks: HttpHooks { clients },
            ctx: WasiHttpCtx::default(),
        })
    }
}

// The shared client and the ones built for a guest's own `connect-timeout`,
// one per distinct value, so a guest that sets one on every request keeps a
// connection pool rather than opening a connection per request. A client is
// `Arc`-backed, so a clone per store shares the pools.
#[derive(Debug, Clone)]
struct Clients {
    shared: reqwest::Client,
    connect_timeout: Duration,
    by_timeout: Arc<Mutex<Vec<(Duration, reqwest::Client)>>>,
}

impl Clients {
    // distinct guest `connect-timeout` values kept; past it a request builds its own
    const CAP: usize = 8;

    fn new(connect_timeout: Duration) -> reqwest::Result<Self> {
        Ok(Self {
            shared: Self::builder(connect_timeout).build()?,
            connect_timeout,
            by_timeout: Arc::default(),
        })
    }

    // what every client starts from, so one built for a request differs from
    // the shared one only in what the request asked for
    fn builder(connect_timeout: Duration) -> reqwest::ClientBuilder {
        reqwest::Client::builder().connect_timeout(connect_timeout)
    }

    // the client for a request's `connect-timeout`: the shared one when the
    // request sets none, or the configured value
    fn get(&self, connect_timeout: Option<Duration>) -> reqwest::Result<reqwest::Client> {
        let Some(timeout) = connect_timeout.filter(|timeout| *timeout != self.connect_timeout)
        else {
            return Ok(self.shared.clone());
        };

        let mut by_timeout = self.by_timeout.lock().expect("clients lock");
        if let Some((_, client)) = by_timeout.iter().find(|(kept, _)| *kept == timeout) {
            return Ok(client.clone());
        }
        let client = Self::builder(timeout).build()?;
        if by_timeout.len() < Self::CAP {
            by_timeout.push((timeout, client.clone()));
        }
        drop(by_timeout);
        Ok(client)
    }

    // a client of the request's own: the identity is part of the TLS configuration
    fn with_identity(
        &self, connect_timeout: Option<Duration>, identity: reqwest::Identity,
    ) -> reqwest::Result<reqwest::Client> {
        Self::builder(connect_timeout.unwrap_or(self.connect_timeout)).identity(identity).build()
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
        let clients = self.clients.clone();

        // guest-supplied timeouts from `wasi:http/types.request-options`
        let opt_connect = options.and_then(|o| o.connect_timeout);
        let opt_first_byte = options.and_then(|o| o.first_byte_timeout);
        let opt_between = options.and_then(|o| o.between_bytes_timeout);

        Box::new(async move {
            let (mut parts, body) = request.into_parts();

            let client = match parts.headers.remove("Client-Cert") {
                Some(encoded_cert) => {
                    tracing::debug!("using client certificate");
                    let encoded = encoded_cert.to_str().map_err(internal_err)?;
                    let bytes = Base64::decode_vec(encoded).map_err(internal_err)?;
                    client_cert::validate_bundle(&bytes).map_err(internal_err)?;
                    let identity = reqwest::Identity::from_pem(&bytes).map_err(internal_err)?;
                    clients.with_identity(opt_connect, identity).map_err(internal_err)?
                }
                None => clients.get(opt_connect).map_err(internal_err)?,
            };

            // stream the outbound body rather than hold it whole in host memory
            let body = reqwest::Body::wrap_stream(
                body.into_data_stream().map_err(|e| io::Error::other(e.to_string())),
            );

            // make request
            let url = parts.uri.to_string();
            let send = client.request(parts.method, &url).headers(parts.headers).body(body).send();

            // the first byte is awaited on top of the connect, which reqwest
            // bounds itself; the streamed body is paced by `between_bytes`
            let resp = match opt_first_byte {
                Some(first_byte) => {
                    let budget =
                        opt_connect.unwrap_or(clients.connect_timeout).saturating_add(first_byte);
                    match tokio::time::timeout(budget, send).await {
                        Ok(result) => result.map_err(|e| reqwest_err(&e))?,
                        Err(_elapsed) => return Err(HttpError::ConnectionReadTimeout),
                    }
                }
                None => send.await.map_err(|e| reqwest_err(&e))?,
            };

            // forbidden headers are stripped by the runtime before the guest sees them
            let converted: Response<reqwest::Body> = resp.into();
            let (parts, body) = converted.into_parts();
            let body = body.map_err(|e| reqwest_err(&e)).boxed_unsync();
            let body = match opt_between {
                Some(between) => Paced::new(body, between).boxed_unsync(),
                None => body,
            };
            let response = Response::from_parts(parts, body);

            Ok((response, fut))
        })
    }
}

// A response body held to the guest's `between-bytes-timeout`: the deadline
// restarts at every frame, and a body silent past it fails as a read timeout.
struct Paced {
    inner: WasiBody,
    between: Duration,
    deadline: Pin<Box<Sleep>>,
}

impl Paced {
    fn new(inner: WasiBody, between: Duration) -> Self {
        Self {
            inner,
            between,
            deadline: Box::pin(tokio::time::sleep(between)),
        }
    }
}

impl Body for Paced {
    type Data = Bytes;
    type Error = HttpError;

    fn poll_frame(
        self: Pin<&mut Self>, cx: &mut task::Context<'_>,
    ) -> Poll<Option<Result<Frame<Bytes>, HttpError>>> {
        let this = self.get_mut();
        match Pin::new(&mut this.inner).poll_frame(cx) {
            Poll::Ready(frame) => {
                this.deadline.as_mut().reset(tokio::time::Instant::now() + this.between);
                Poll::Ready(frame)
            }
            Poll::Pending => {
                ready!(this.deadline.as_mut().poll(cx));
                Poll::Ready(Some(Err(HttpError::ConnectionReadTimeout)))
            }
        }
    }

    fn is_end_stream(&self) -> bool {
        self.inner.is_end_stream()
    }

    fn size_hint(&self) -> SizeHint {
        self.inner.size_hint()
    }
}

fn internal_err(e: impl Display) -> HttpError {
    HttpError::InternalError(Some(e.to_string()))
}

// reqwest reports one error whose source chain holds the cause, so the phase
// is read off the chain: a connect or TLS failure hands its io error on to
// wasmtime's own classification (dns, unreachable, timed out, terminated,
// ...) rather than flattening every connection failure to `connection-refused`
fn reqwest_err(e: &reqwest::Error) -> HttpError {
    if e.is_timeout() {
        return if e.is_connect() {
            HttpError::ConnectionTimeout
        } else {
            HttpError::ConnectionReadTimeout
        };
    }
    if e.is_connect() {
        return cause::<io::Error>(e).map_or(HttpError::ConnectionRefused, connect_err);
    }
    if e.is_builder() {
        return HttpError::HttpRequestUriInvalid;
    }
    match cause::<hyper::Error>(e) {
        Some(hyper) if hyper.is_incomplete_message() => HttpError::HttpResponseIncomplete,
        Some(_) => HttpError::HttpProtocolError,
        None => internal_err(e),
    }
}

// the first `T` on the chain: reqwest wraps hyper-util's connect error, whose
// source is the resolver's or the socket's io error, or the TLS handshake's,
// and hyper's own error for a failure once connected
fn cause<'a, T: Error + 'static>(e: &'a (dyn Error + 'static)) -> Option<&'a T> {
    let mut cause = Some(e);
    while let Some(err) = cause {
        if let Some(found) = err.downcast_ref::<T>() {
            return Some(found);
        }
        cause = err.source();
    }
    None
}

fn connect_err(io: &io::Error) -> HttpError {
    let Some(handshake) = io.get_ref().and_then(|inner| inner.downcast_ref::<io::Error>()) else {
        return HttpError::Connect(io_copy(io));
    };
    match handshake.get_ref().and_then(|inner| inner.downcast_ref::<rustls::Error>()) {
        Some(rustls::Error::InvalidCertificate(_)) => HttpError::TlsCertificateError,
        Some(rustls::Error::AlertReceived(alert)) => HttpError::TlsAlertReceived {
            alert_id: Some(u8::from(*alert)),
            alert_message: None,
        },
        _ => HttpError::Tls(io_copy(handshake)),
    }
}

fn io_copy(io: &io::Error) -> io::Error {
    io.raw_os_error()
        .map_or_else(|| io::Error::new(io.kind(), io.to_string()), io::Error::from_raw_os_error)
}
