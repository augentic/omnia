//! HTTP handlers over plain axum.
//!
//! A route is an ordinary axum handler on an `axum::Router` whose state is
//! one provider-owning [`Client`]. The handler reaches its capabilities
//! through the [`Context`] extractor, reads its input with axum's `Path`,
//! `Query`, and `Json` extractors, and answers a `Result` whose error is an
//! [`Error`](crate::Error), the JSON [`ErrorBody`] at its status; `?` on an
//! `anyhow::Error` reaches the same envelope. A domain error with a wire shape
//! of its own implements `IntoResponse` itself. A module builds its routes
//! as a [`Routes`] of its own and the guest merges them (`Routes::from([..])`
//! or [`Routes::merge`]) into its one [`Router`] with
//! `with_state(Client::new(provider))`. [`export!`] binds the router fn as
//! the guest's `wasi:http` handler and serves each request inside the
//! `handler` span [`trace`] opens.

use std::convert::Infallible;
use std::fmt;
use std::future::{Future, ready};
use std::sync::Arc;

use axum::Json;
use axum::extract::{FromRef, FromRequestParts, Request};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::routing::MethodRouter;
use http::HeaderMap;
use http::request::Parts;
use tracing::Instrument as _;

use crate::api::{Client, Context, ErrorBody, Metadata};

#[doc(hidden)]
#[macro_export]
macro_rules! __export_http {
    ($router:path $(,)?) => {
        $crate::api::http::export!($router, $crate::WasiProvider);
    };
    ($router:path, $provider:expr $(,)?) => {
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
                        $router($provider),
                        $crate::axum::middleware::from_fn($crate::api::http::trace),
                    );
                    $crate::api::http::serve(router, request).await
                }
            }
        };
    };
}

/// Bind a router fn as the guest's `wasi:http` export.
///
/// Calls the router fn on each request with [`WasiProvider`](crate::WasiProvider),
/// or with the provider expression passed as a second argument, and serves the
/// request through the resulting `axum::Router` inside the `handler` span
/// [`trace`] opens. Expands to nothing on native targets, so the same crate's
/// tests build the router natively.
///
/// ```rust,ignore
/// use omnia_sdk::api::http;
///
/// http::export!(http_router);
/// ```
#[doc(inline)]
pub use crate::__export_http as export;

/// HTTP routes over a provider-owning [`Client`], awaiting their state.
///
/// A module registers its paths on one of these and the guest merges them
/// (`Routes::from([..])` or [`merge`](Self::merge)), closing the whole with
/// [`with_state`](Self::with_state) into a [`Router`], as topics are
/// [`messaging::Routes`](crate::api::messaging::Routes) closed into a
/// [`messaging::Router`](crate::api::messaging::Router).
#[must_use = "routes serve nothing until closed into a `Router` with `with_state`"]
pub struct Routes<P: Send + Sync + 'static>(axum::Router<Client<P>>);

impl<P: Send + Sync + 'static> Default for Routes<P> {
    fn default() -> Self {
        Self::new()
    }
}

impl<P: Send + Sync + 'static> Routes<P> {
    /// Create an empty set of routes.
    pub fn new() -> Self {
        Self(axum::Router::new())
    }

    /// Register a [`MethodRouter`] at `path`.
    ///
    /// # Panics
    ///
    /// Panics when `path` overlaps an existing route for the same method.
    pub fn route(self, path: &str, method_router: MethodRouter<Client<P>>) -> Self {
        Self(self.0.route(path, method_router))
    }

    /// Take every route of `other`.
    ///
    /// # Panics
    ///
    /// Panics when a path of `other` overlaps an existing route for the same method.
    pub fn merge(self, other: Self) -> Self {
        Self(self.0.merge(other.0))
    }

    /// Close the routes over `client` as the [`Router`] that serves them.
    pub fn with_state(self, client: Client<P>) -> Router {
        self.0.with_state(client)
    }
}

impl<P: Send + Sync + 'static, const N: usize> From<[Self; N]> for Routes<P> {
    fn from(parts: [Self; N]) -> Self {
        parts.into_iter().reduce(Self::merge).unwrap_or_default()
    }
}

impl<P: Send + Sync + 'static> From<Routes<P>> for axum::Router<Client<P>> {
    fn from(routes: Routes<P>) -> Self {
        routes.0
    }
}

impl<P: Send + Sync + 'static> Clone for Routes<P> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl<P: Send + Sync + 'static> fmt::Debug for Routes<P> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

/// A finished `axum::Router` ready to serve requests, or one over state `S`.
pub type Router<S = ()> = axum::Router<S>;

/// Serve a request inside a `handler` span carrying its request and correlation ids.
///
/// Apply with `axum::middleware::from_fn(trace)`; [`export!`] applies it to
/// the router it exports. The ids the span carries are the ones the
/// [`Metadata`] and [`Context`] extractors hand the handler, a minted request
/// id included.
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
