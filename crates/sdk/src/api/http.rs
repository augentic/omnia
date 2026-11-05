//! HTTP handlers and error responses over plain axum.
//!
//! A route is an ordinary axum handler on an [`axum::Router`] whose state is
//! one provider-owning [`Client`]. The handler reaches its capabilities
//! through the [`Context`] extractor, reads its input with axum's `Path`,
//! `Query`, and `Json` extractors, and answers a `Result` whose error is an
//! [`Error`](crate::Error) or an [`HttpError`]: an `Error` is the JSON
//! [`ErrorBody`] at its status, and `?` on an `anyhow::Error` reaches the
//! same envelope through either. [`http!`] exports the finished router as the
//! guest's `wasi:http` handler and serves each request inside the `handler`
//! span [`trace`] opens. Omnia creates one component instance per HTTP
//! request, so the router and client are constructed inside the export;
//! durable state belongs in host-side capabilities.

use std::convert::Infallible;
use std::future::{Future, ready};
use std::sync::Arc;

use axum::extract::{FromRef, FromRequestParts, Request};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use http::header::CONTENT_TYPE;
use http::request::Parts;
use http::{HeaderMap, HeaderValue, StatusCode};
use tracing::Instrument as _;

pub use crate::api::DecodeError;
use crate::api::{Client, Context, Encoded, ErrorBody, Metadata};

/// Result type for HTTP handlers whose failures carry their own wire body.
pub type HttpResult<T, E = HttpError> = Result<T, E>;

/// Serve a request inside a `handler` span carrying its request and correlation ids.
///
/// Apply with `axum::middleware::from_fn(trace)`; [`http!`] applies it to the
/// router it exports. The ids the span carries are the ones the [`Metadata`]
/// and [`Context`] extractors hand the handler, a minted request id included.
pub async fn trace(mut request: Request, next: Next) -> Response {
    let metadata = Metadata::from_headers(request.headers());
    let span = tracing::info_span!(
        "handler",
        request_id = metadata.request_id.as_deref(),
        correlation_id = metadata.correlation_id.as_deref(),
    );
    request.extensions_mut().insert(metadata);
    next.run(request).instrument(span).await
}

impl Metadata {
    /// Build metadata from HTTP request headers: `x-request-id`,
    /// `x-correlation-id`, and `x-causation-id`.
    ///
    /// HTTP's carrier for the ids the command line reads from the environment;
    /// a missing request id is minted as on every transport.
    #[must_use]
    pub fn from_headers(headers: &HeaderMap) -> Self {
        Self::from_lookup(|name| {
            headers
                .get(format!("x-{name}"))
                .and_then(|value| value.to_str().ok())
                .map(str::to_owned)
        })
    }
}

// Reads the metadata `trace` stored, so a request id minted there is minted once.
impl<S: Send + Sync> FromRequestParts<S> for Metadata {
    type Rejection = Infallible;

    fn from_request_parts(
        parts: &mut Parts, _state: &S,
    ) -> impl Future<Output = Result<Self, Self::Rejection>> + Send {
        ready(Ok(parts
            .extensions
            .get::<Self>()
            .cloned()
            .unwrap_or_else(|| Self::from_headers(&parts.headers))))
    }
}

impl<P, S> FromRequestParts<S> for Context<P>
where
    P: Send + Sync + 'static,
    S: Send + Sync,
    Client<P>: FromRef<S>,
{
    type Rejection = Infallible;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let client = Client::from_ref(state);
        let metadata = Metadata::from_request_parts(parts, state).await?;
        Ok(Self {
            owner: Arc::clone(&client.owner),
            provider: Arc::clone(&client.provider),
            metadata,
        })
    }
}

/// An HTTP error response: a status and a body with its content type.
///
/// An [`Error`](crate::Error), a [`DecodeError`], and an `anyhow::Error`
/// convert into the JSON [`ErrorBody`] envelope at the error's status, so a
/// handler answering [`HttpResult`] reaches it with `?`. [`HttpError::with_body`]
/// carries a preformatted body in any other wire format.
#[derive(Debug)]
pub struct HttpError {
    status: StatusCode,
    content_type: HeaderValue,
    body: Vec<u8>,
}

impl HttpError {
    /// Create a plain-text HTTP error.
    #[must_use]
    pub fn new(status: StatusCode, message: impl Into<String>) -> Self {
        Self::with_body(
            status,
            HeaderValue::from_static("text/plain; charset=utf-8"),
            message.into().into_bytes(),
        )
    }

    /// Create an HTTP error carrying a preformatted wire body.
    #[must_use]
    pub const fn with_body(status: StatusCode, content_type: HeaderValue, body: Vec<u8>) -> Self {
        Self {
            status,
            content_type,
            body,
        }
    }
}

// Carries the same `error` discriminant every transport emits, so a client
// classifies failures identically over HTTP and the command line.
impl From<crate::Error> for HttpError {
    fn from(error: crate::Error) -> Self {
        let body =
            serde_json::to_vec(&ErrorBody::from(&error)).expect("a two-string struct serializes");
        Self::with_body(error.status(), HeaderValue::from_static("application/json"), body)
    }
}

impl From<DecodeError> for HttpError {
    fn from(error: DecodeError) -> Self {
        crate::Error::BadRequest {
            code: "invalid_request".to_string(),
            description: error.description().to_owned(),
        }
        .into()
    }
}

impl From<anyhow::Error> for HttpError {
    fn from(error: anyhow::Error) -> Self {
        crate::Error::from(error).into()
    }
}

impl IntoResponse for HttpError {
    fn into_response(self) -> Response {
        (self.status, [(CONTENT_TYPE, self.content_type)], self.body).into_response()
    }
}

// Lets a handler answer `Result<_, Error>` directly, with the same envelope.
impl IntoResponse for crate::Error {
    fn into_response(self) -> Response {
        HttpError::from(self).into_response()
    }
}

impl IntoResponse for Encoded {
    fn into_response(self) -> Response {
        (StatusCode::OK, [(CONTENT_TYPE, HeaderValue::from_static(self.media_type))], self.bytes)
            .into_response()
    }
}

/// Wires a finished [`axum::Router`] as the guest's `wasi:http/incoming-handler` export.
///
/// Each request is served inside an `http_guest_handle` span and the `handler`
/// span [`trace`](crate::api::http::trace) opens; expands to nothing on native targets.
#[macro_export]
macro_rules! http {
    ($router:expr) => {
        #[cfg(all(target_arch = "wasm32", target_os = "wasi"))]
        const _: () = {
            use $crate::omnia_wasi_otel;

            struct HttpGuest;

            $crate::wasip3::http::service::export!(HttpGuest);

            impl $crate::wasip3::exports::http::handler::Guest for HttpGuest {
                #[omnia_wasi_otel::instrument(name = "http_guest_handle", level = $crate::tracing::Level::INFO)]
                async fn handle(
                    request: $crate::wasip3::http::types::Request,
                ) -> ::core::result::Result<
                    $crate::wasip3::http::types::Response,
                    $crate::wasip3::http::types::ErrorCode,
                > {
                    let router = $crate::axum::Router::layer(
                        $router,
                        $crate::axum::middleware::from_fn($crate::api::http::trace),
                    );
                    $crate::api::http::serve(router, request).await
                }
            }
        };
    };
}

/// Serve one WASI HTTP request through a finished [`axum::Router`].
#[cfg(all(target_arch = "wasm32", target_os = "wasi"))]
pub use omnia_wasi_http::serve;
