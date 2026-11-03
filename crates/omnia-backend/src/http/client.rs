//! Service contract for outbound HTTP backends.

use std::fmt::Debug;
use std::io;
use std::time::Duration;

use bytes::Bytes;
use futures::future::BoxFuture;
use http::{Request, Response};
use http_body_util::combinators::UnsyncBoxBody;

/// Streaming HTTP body used by [`HttpClient::fetch`].
pub type Body = UnsyncBoxBody<Bytes, Error>;

/// Per-request timeout options for [`HttpClient::fetch`].
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct RequestOptions {
    /// How long to wait for a connection to be established.
    pub connect_timeout: Option<Duration>,
    /// How long to wait for the first byte of the response body.
    pub first_byte_timeout: Option<Duration>,
    /// How long to wait between frames of the response body.
    pub between_bytes_timeout: Option<Duration>,
}

/// Typed outbound HTTP failure.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// Connection I/O error.
    #[error("connection error: {0}")]
    Connect(#[source] io::Error),
    /// TLS handshake I/O error.
    #[error("TLS error: {0}")]
    Tls(#[source] io::Error),
    /// Connection refused by peer.
    #[error("connection refused")]
    ConnectionRefused,
    /// Connect timeout elapsed.
    #[error("connection timeout")]
    ConnectionTimeout,
    /// Read or first-byte timeout elapsed.
    #[error("connection read timeout")]
    ConnectionReadTimeout,
    /// Invalid peer certificate.
    #[error("TLS certificate error")]
    TlsCertificateError,
    /// Peer sent a TLS alert.
    #[error("TLS alert received")]
    TlsAlertReceived {
        /// TLS alert code, when known.
        alert_id: Option<u8>,
        /// Optional alert message.
        alert_message: Option<String>,
    },
    /// Request URI could not be built.
    #[error("invalid HTTP request URI")]
    HttpRequestUriInvalid,
    /// Response message ended before completion.
    #[error("incomplete HTTP response")]
    HttpResponseIncomplete,
    /// HTTP protocol violation.
    #[error("HTTP protocol error")]
    HttpProtocolError,
    /// Internal client failure.
    #[error("internal error: {}", .0.as_deref().unwrap_or("unspecified"))]
    InternalError(Option<String>),
}

/// Service contract for outbound HTTP backends.
pub trait HttpClient: Debug + Send + Sync + 'static {
    /// Execute an outbound HTTP `request` with optional `options`.
    fn fetch(
        &self, request: Request<Body>, options: Option<RequestOptions>,
    ) -> BoxFuture<'static, Result<Response<Body>, Error>>;
}
