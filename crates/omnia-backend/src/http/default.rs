//! Default reqwest-backed implementation of [`HttpClient`].

use std::error::Error as StdError;
use std::fmt::{Debug, Display};
use std::io;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{self, Poll, ready};
use std::time::Duration;

use anyhow::Context;
use base64ct::{Base64, Encoding};
use bytes::Bytes;
use futures::future::BoxFuture;
use futures::{FutureExt as _, TryStreamExt as _};
use http::{Request, Response};
use http_body::{Body as HttpBody, Frame, SizeHint};
use http_body_util::BodyExt as _;
use tokio::time::Sleep;
use tracing::instrument;

use super::{Body, Error, HttpClient, RequestOptions, client_cert};
use crate::Connect;

#[expect(missing_docs, reason = "`FromEnv` has no docs")]
mod config {
    use fromenv::FromEnv;

    /// Options for the default outbound HTTP client.
    #[derive(Debug, Clone, FromEnv)]
    pub struct ConnectOptions {
        /// Connect timeout in seconds (`HTTP_CONNECT_TIMEOUT`, default 10).
        #[env(from = "HTTP_CONNECT_TIMEOUT", default = "10")]
        pub connect_timeout: u64,
    }
}
pub use config::ConnectOptions;

impl crate::FromEnv for ConnectOptions {
    fn load_env() -> anyhow::Result<Self> {
        Self::from_env().finalize().context("issue loading connection options")
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

/// Default reqwest-backed implementation of [`HttpClient`].
#[derive(Debug, Clone)]
pub struct HttpDefault {
    clients: Clients,
}

impl Connect for HttpDefault {
    type ConnectOptions = ConnectOptions;

    #[instrument]
    async fn connect_with(options: Self::ConnectOptions) -> anyhow::Result<Self> {
        ensure_crypto_provider();
        let clients = Clients::new(Duration::from_secs(options.connect_timeout))
            .context("building HTTP client")?;
        Ok(Self { clients })
    }
}

impl HttpClient for HttpDefault {
    fn fetch(
        &self, request: Request<Body>, options: Option<RequestOptions>,
    ) -> BoxFuture<'static, Result<Response<Body>, Error>> {
        let clients = self.clients.clone();
        let opt_connect = options.and_then(|o| o.connect_timeout);
        let opt_first_byte = options.and_then(|o| o.first_byte_timeout);
        let opt_between = options.and_then(|o| o.between_bytes_timeout);

        async move {
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

            let body = reqwest::Body::wrap_stream(
                body.into_data_stream().map_err(|e| io::Error::other(e.to_string())),
            );

            let url = parts.uri.to_string();
            let send = client.request(parts.method, &url).headers(parts.headers).body(body).send();

            let resp = match opt_first_byte {
                Some(first_byte) => {
                    let budget =
                        opt_connect.unwrap_or(clients.connect_timeout).saturating_add(first_byte);
                    match tokio::time::timeout(budget, send).await {
                        Ok(result) => result.map_err(|e| reqwest_err(&e))?,
                        Err(_elapsed) => return Err(Error::ConnectionReadTimeout),
                    }
                }
                None => send.await.map_err(|e| reqwest_err(&e))?,
            };

            let converted: Response<reqwest::Body> = resp.into();
            let (parts, body) = converted.into_parts();
            let body = body.map_err(|e| reqwest_err(&e)).boxed_unsync();
            let body = match opt_between {
                Some(between) => Paced::new(body, between).boxed_unsync(),
                None => body,
            };
            Ok(Response::from_parts(parts, body))
        }
        .boxed()
    }
}

#[derive(Debug, Clone)]
struct Clients {
    shared: reqwest::Client,
    connect_timeout: Duration,
    by_timeout: Arc<Mutex<Vec<(Duration, reqwest::Client)>>>,
}

impl Clients {
    const CAP: usize = 8;

    fn new(connect_timeout: Duration) -> reqwest::Result<Self> {
        Ok(Self {
            shared: Self::builder(connect_timeout).build()?,
            connect_timeout,
            by_timeout: Arc::default(),
        })
    }

    fn builder(connect_timeout: Duration) -> reqwest::ClientBuilder {
        reqwest::Client::builder().connect_timeout(connect_timeout)
    }

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

    fn with_identity(
        &self, connect_timeout: Option<Duration>, identity: reqwest::Identity,
    ) -> reqwest::Result<reqwest::Client> {
        Self::builder(connect_timeout.unwrap_or(self.connect_timeout)).identity(identity).build()
    }
}

struct Paced {
    inner: Body,
    between: Duration,
    deadline: Pin<Box<Sleep>>,
}

impl Paced {
    fn new(inner: Body, between: Duration) -> Self {
        Self {
            inner,
            between,
            deadline: Box::pin(tokio::time::sleep(between)),
        }
    }
}

impl HttpBody for Paced {
    type Data = Bytes;
    type Error = Error;

    fn poll_frame(
        self: Pin<&mut Self>, cx: &mut task::Context<'_>,
    ) -> Poll<Option<Result<Frame<Bytes>, Error>>> {
        let this = self.get_mut();
        match Pin::new(&mut this.inner).poll_frame(cx) {
            Poll::Ready(frame) => {
                this.deadline.as_mut().reset(tokio::time::Instant::now() + this.between);
                Poll::Ready(frame)
            }
            Poll::Pending => {
                ready!(this.deadline.as_mut().poll(cx));
                Poll::Ready(Some(Err(Error::ConnectionReadTimeout)))
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

fn internal_err(e: impl Display) -> Error {
    Error::InternalError(Some(e.to_string()))
}

fn reqwest_err(e: &reqwest::Error) -> Error {
    if e.is_timeout() {
        return if e.is_connect() {
            Error::ConnectionTimeout
        } else {
            Error::ConnectionReadTimeout
        };
    }
    if e.is_connect() {
        return cause::<io::Error>(e).map_or(Error::ConnectionRefused, connect_err);
    }
    if e.is_builder() {
        return Error::HttpRequestUriInvalid;
    }
    match cause::<hyper::Error>(e) {
        Some(hyper) if hyper.is_incomplete_message() => Error::HttpResponseIncomplete,
        Some(_) => Error::HttpProtocolError,
        None => internal_err(e),
    }
}

fn cause<'a, T: StdError + 'static>(e: &'a (dyn StdError + 'static)) -> Option<&'a T> {
    let mut cause = Some(e);
    while let Some(err) = cause {
        if let Some(found) = err.downcast_ref::<T>() {
            return Some(found);
        }
        cause = err.source();
    }
    None
}

fn connect_err(io: &io::Error) -> Error {
    let Some(handshake) = io.get_ref().and_then(|inner| inner.downcast_ref::<io::Error>()) else {
        return Error::Connect(io_copy(io));
    };
    match handshake.get_ref().and_then(|inner| inner.downcast_ref::<rustls::Error>()) {
        Some(rustls::Error::InvalidCertificate(_)) => Error::TlsCertificateError,
        Some(rustls::Error::AlertReceived(alert)) => Error::TlsAlertReceived {
            alert_id: Some(u8::from(*alert)),
            alert_message: None,
        },
        _ => Error::Tls(io_copy(handshake)),
    }
}

fn io_copy(io: &io::Error) -> io::Error {
    io.raw_os_error()
        .map_or_else(|| io::Error::new(io.kind(), io.to_string()), io::Error::from_raw_os_error)
}
