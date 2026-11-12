//! HTTP handlers over plain axum.
//!
//! A route is an ordinary axum handler on an `axum::Router` whose state is
//! one provider-owning [`Client`]. The handler reaches its capabilities
//! through the [`Context`] extractor, reads its input with axum's `Path`,
//! `Query`, and `Json` extractors, and answers a `Result` whose error is an
//! [`Error`](crate::Error), the JSON [`ErrorBody`] at its status; `?` on an
//! `anyhow::Error` reaches the same envelope. A domain error with a wire shape
//! of its own implements `IntoResponse` itself. A module builds its routes
//! and topics on a [`Handlers`] of its own and the guest merges them
//! (`Handlers::from([..])` or [`Handlers::merge`]) into its one
//! [`api::Router`](Router) with `with_state(Client::new(provider))`.
//! [`export!`] binds the router fn as the guest's `wasi:http` handler and
//! serves each request inside the `handler` span [`trace`] opens.

use std::convert::Infallible;
use std::future::{Future, ready};
use std::sync::Arc;

use axum::Json;
use axum::extract::{FromRef, FromRequestParts, MatchedPath, Request};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::routing::MethodRouter;
use http::HeaderMap;
use http::request::Parts;
use tracing::Instrument as _;

use crate::api::{Client, Context, ErrorBody, Handlers, Metadata, Router};

// What `Handlers` and `Router` hold for HTTP.
pub(crate) type Routes<P> = axum::Router<Client<P>>;
pub(crate) type Served = axum::Router;

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
                        $crate::axum::Router::from($router($provider)),
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
/// request through the resulting router inside the `handler` span [`trace`]
/// opens. Expands to nothing on native targets, where a test or a native host
/// serves the router itself.
///
/// ```rust,ignore
/// use omnia_sdk::api::http;
///
/// http::export!(http_router);
/// ```
#[doc(inline)]
pub use crate::__export_http as export;

/// Serve a request inside a `handler` span carrying its request and
/// correlation ids, its method, and its route.
///
/// Apply with `axum::middleware::from_fn(trace)`; [`export!`] applies it to
/// the router it exports. The ids the span carries are the ones the
/// [`Metadata`] and [`Context`] extractors hand the handler, a minted request
/// id included. The `route` is the template the router matched, such as
/// `/items/{id}`; a request no route matched carries none.
pub async fn trace(mut request: Request, next: Next) -> Response {
    let metadata = Metadata::from_headers(request.headers());
    let span = tracing::info_span!(
        "handler",
        request_id = metadata.request_id.as_deref(),
        correlation_id = metadata.correlation_id.as_deref(),
        method = request.method().as_str(),
        route = request.extensions().get::<MatchedPath>().map(MatchedPath::as_str),
    );
    request.extensions_mut().insert(metadata);
    next.run(request).instrument(span).await
}

/// Serve one WASI HTTP request through a finished [`axum::Router`].
#[cfg(all(target_arch = "wasm32", target_os = "wasi"))]
pub use omnia_wasi_http::serve;

impl Metadata {
    /// Build metadata from HTTP request headers: `x-request-id`,
    /// `x-correlation-id`, `x-causation-id`, `x-tenant`, and `x-deadline`.
    ///
    /// HTTP's carrier for the values the command line reads from the
    /// environment; a missing request id is minted as on every transport.
    ///
    /// # Panics
    ///
    /// Panics as [`Metadata::from_lookup`] does.
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

impl<P: Send + Sync + 'static> Handlers<P> {
    /// Register a [`MethodRouter`] at `path`.
    ///
    /// # Panics
    ///
    /// Panics when `path` overlaps an existing route for the same method.
    pub fn route(mut self, path: &str, method_router: MethodRouter<Client<P>>) -> Self {
        self.http = self.http.route(path, method_router);
        self
    }
}

impl<P: Send + Sync + 'static> From<Handlers<P>> for axum::Router<Client<P>> {
    fn from(handlers: Handlers<P>) -> Self {
        handlers.http
    }
}

impl<P: Send + Sync + 'static> Router<P> {
    /// Apply a [`tower::Layer`] to every HTTP route on this router.
    pub fn layer<L>(mut self, layer: L) -> Self
    where
        L: tower::Layer<axum::routing::Route> + Clone + Send + Sync + 'static,
        L::Service:
            tower::Service<::http::Request<axum::body::Body>> + Clone + Send + Sync + 'static,
        <L::Service as tower::Service<::http::Request<axum::body::Body>>>::Response:
            axum::response::IntoResponse + 'static,
        <L::Service as tower::Service<::http::Request<axum::body::Body>>>::Error:
            Into<std::convert::Infallible> + 'static,
        <L::Service as tower::Service<::http::Request<axum::body::Body>>>::Future: Send + 'static,
    {
        self.http = self.http.layer(layer);
        self
    }
}

impl<P: Send + Sync + 'static> From<Router<P>> for axum::Router {
    fn from(router: Router<P>) -> Self {
        router.http
    }
}

impl<P, B> tower::Service<::http::Request<B>> for Router<P>
where
    P: Send + Sync + 'static,
    B: http_body::Body<Data = bytes::Bytes> + Send + 'static,
    B::Error: Into<axum::BoxError>,
{
    type Error = std::convert::Infallible;
    type Future = axum::routing::future::RouteFuture<std::convert::Infallible>;
    type Response = axum::response::Response;

    fn poll_ready(
        &mut self, cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Result<(), Self::Error>> {
        <axum::Router as tower::Service<::http::Request<B>>>::poll_ready(&mut self.http, cx)
    }

    fn call(&mut self, req: ::http::Request<B>) -> Self::Future {
        self.http.call(req)
    }
}

// Carries the same `error` discriminant every transport emits, so a client
// classifies failures identically over HTTP and the command line.
impl IntoResponse for crate::Error {
    fn into_response(self) -> Response {
        (self.status(), Json(ErrorBody::from(&self))).into_response()
    }
}
