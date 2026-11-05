//! HTTP handlers over plain axum.
//!
//! A route is an ordinary axum handler on an [`axum::Router`] whose state is
//! one provider-owning [`Client`]. The handler reaches its capabilities
//! through the [`Context`] extractor, reads its input with axum's `Path`,
//! `Query`, and `Json` extractors, and answers a `Result` whose error is an
//! [`Error`](crate::Error), the JSON [`ErrorBody`] at its status; `?` on an
//! `anyhow::Error` reaches the same envelope. A domain error with a wire shape
//! of its own implements `IntoResponse` itself. [`export!`](crate::export)
//! binds the finished router as the guest's `wasi:http` handler and serves
//! each request inside the `handler` span [`trace`] opens.

use std::convert::Infallible;
use std::future::{Future, ready};
use std::sync::Arc;

use axum::Json;
use axum::extract::{FromRef, FromRequestParts, Request};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use http::HeaderMap;
use http::request::Parts;
use tracing::Instrument as _;

use crate::api::{Client, Context, ErrorBody, Metadata};

/// Serve a request inside a `handler` span carrying its request and correlation ids.
///
/// Apply with `axum::middleware::from_fn(trace)`; [`export!`](crate::export)
/// applies it to the router it exports. The ids the span carries are the ones
/// the [`Metadata`] and [`Context`] extractors hand the handler, a minted
/// request id included.
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
    /// `x-correlation-id`, `x-causation-id`, and `x-tenant`.
    ///
    /// HTTP's carrier for the values the command line reads from the
    /// environment; a missing request id is minted as on every transport.
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
            provider: Arc::clone(&client.provider),
            metadata,
        })
    }
}

// Carries the same `error` discriminant every transport emits, so a client
// classifies failures identically over HTTP and the command line.
impl IntoResponse for crate::Error {
    fn into_response(self) -> Response {
        (self.status(), Json(ErrorBody::from(&self))).into_response()
    }
}

/// Serve one WASI HTTP request through a finished [`axum::Router`].
#[cfg(all(target_arch = "wasm32", target_os = "wasi"))]
pub use omnia_wasi_http::serve;
