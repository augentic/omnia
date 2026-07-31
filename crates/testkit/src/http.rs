//! Driving a guest's `wasi:http/handler` export in-process.
//!
//! [`HttpHarness`] mirrors the runtime's HTTP trigger server
//! (`crates/wasi-http/src/host/server.rs`) — snapshot the trigger router once
//! at construction (the analogue of the server's boot), then per request
//! resolve the guest by path (static route first, the deployment's
//! `http_paths` hook second, through the runtime's shared `route_http`
//! helper), instantiate it fresh, hand it a `wasi:http` request, and convert
//! the response back — but skips the TCP socket and collects the response
//! body eagerly so a test can assert on it directly. The free helpers
//! ([`handle`], [`get`], [`post`], [`delete`], [`post_json`]) wrap a
//! single-use harness; a scenario spanning several requests (e.g. dynamic
//! registration through the deployment's hook) must drive one harness so
//! routing keeps the production server's boot-frozen lifetime.

use std::sync::Arc;

use anyhow::{Context as _, Result, anyhow, ensure};
use bytes::Bytes;
use http_body_util::{BodyExt as _, Full};
use omnia::{Guest, HttpRoutes, RoutingPolicy, Runtime, StoreCtx, TriggerRouter};
use wasmtime_wasi_http::p3::WasiHttpView;
use wasmtime_wasi_http::p3::bindings::ServiceIndices;
use wasmtime_wasi_http::p3::bindings::http::types::{self as wasi};

/// An in-process stand-in for the HTTP trigger server: routing is built once
/// at construction and reused across requests, exactly like the server's boot.
pub struct HttpHarness<B>
where
    B: Clone + Send + Sync + 'static,
    StoreCtx<B>: WasiHttpView,
{
    runtime: Runtime<B>,
    routing: TriggerRouter<ServiceIndices, HttpRoutes>,
}

impl<B> HttpHarness<B>
where
    B: Clone + Send + Sync + 'static,
    StoreCtx<B>: WasiHttpView,
{
    /// Snapshot the trigger router — the analogue of the HTTP server's boot.
    ///
    /// # Errors
    ///
    /// Returns an error if the router cannot be built, or if no guest exports
    /// the HTTP handler and no `http_paths` hook is installed (mirroring the
    /// server's inert check).
    pub fn new(runtime: Runtime<B>) -> Result<Self> {
        // The runtime builds the router under its deployment's routing
        // policy, so the harness cannot drift from the production server's
        // boot.
        let routing = runtime.http_trigger_router(ServiceIndices::new)?;
        ensure!(
            !routing.is_inert() || runtime.http_routing_policy() == RoutingPolicy::TableOnly,
            "no guest exports the http handler"
        );
        Ok(Self { runtime, routing })
    }

    /// Drive one request through the two-tier path (static route, then the
    /// deployment's `http_paths` hook) and return its fully-collected
    /// response.
    ///
    /// # Errors
    ///
    /// Returns an error if the request path stays unmatched (`no route
    /// matched path`, the server's 404), a claimed path faults (`cannot be
    /// served`, the server's 500), the guest traps or returns an error, or
    /// the response cannot be converted and collected.
    pub async fn handle(&self, request: http::Request<Bytes>) -> Result<http::Response<Bytes>> {
        let path = request.uri().path().to_owned();
        let guest: Arc<Guest<StoreCtx<B>>>;
        let late_indices;
        let indices: &ServiceIndices = if let Some((guest_id, indices)) =
            self.routing.resolve(&path)
        {
            guest =
                self.runtime.registry().get(guest_id).context("resolved guest is registered")?;
            indices
        } else {
            // Static miss: the deployment's `http_paths` hook → ensure
            // (resolve-on-miss) → request-local indices, through the same
            // shared helper as the production server, so the refusal
            // semantics (ordinary 404 vs claimed-path 500) cannot drift.
            // `RouteRefusal`'s `Display`/`source` carry the 404/500 wording
            // the seam assertions match on.
            guest =
                self.runtime.route_http(&path).await.map_err(|refusal| {
                    anyhow::Error::new(refusal).context(format!("path `{path}`"))
                })?;
            late_indices = ServiceIndices::new(guest.instance_pre())
                .map_err(anyhow::Error::from)
                .with_context(|| {
                format!("claimed route `{path}` guest lacks the http handler")
            })?;
            &late_indices
        };

        // Instantiate fresh (instance-per-call) and load the typed handler.
        let mut store = self.runtime.build_store(self.runtime.store());
        let instance = self.runtime.instantiate(guest.instance_pre(), &mut store).await?;
        let service = indices.load(&mut store, &instance)?;

        // `Full<Bytes>` has `Error = Infallible`, which `wasi:http` converts to
        // `ErrorCode` for free — so no error mapping is needed on the request body.
        let (parts, body) = request.into_parts();
        let http_req = http::Request::from_parts(parts, Full::new(body));

        let response = store
            .run_concurrent(async move |store| {
                let (request, io) = wasi::Request::from_http(http_req);
                let wasi_resp = service
                    .handle(store, request)
                    .await
                    .map_err(anyhow::Error::from)
                    .context("guest trap")?
                    .map_err(|error| anyhow!("guest error: {error}"))?;
                let http_resp = store
                    .with(|mut store| wasi_resp.into_http(&mut store, io))
                    .map_err(|error| anyhow!("converting guest response: {error}"))?;

                // Collect the (possibly streaming) body here, while `run_concurrent`
                // still drives the instance's tasks; the WASI pipe resources are
                // valid only inside this closure.
                let (parts, body) = http_resp.into_parts();
                let collected = body
                    .collect()
                    .await
                    .map_err(|error| anyhow!("reading guest response body: {error:?}"))?;
                anyhow::Ok(http::Response::from_parts(parts, collected.to_bytes()))
            })
            .await??;

        Ok(response)
    }

    /// Drive a `GET {path}` request through the harness.
    ///
    /// # Errors
    ///
    /// Propagates any error from [`handle`](Self::handle), plus a
    /// request-construction error.
    pub async fn get(&self, path: &str) -> Result<http::Response<Bytes>> {
        self.handle(get_request(path)?).await
    }

    /// Drive a `POST {path}` request carrying `body` through the harness.
    ///
    /// # Errors
    ///
    /// Propagates any error from [`handle`](Self::handle), plus a
    /// request-construction error.
    pub async fn post(&self, path: &str, body: impl Into<Bytes>) -> Result<http::Response<Bytes>> {
        self.handle(post_request(path, body)?).await
    }
}

/// Drive one request through the runtime's `wasi:http` guest and return its
/// fully-collected response (a single-use [`HttpHarness`]).
///
/// # Errors
///
/// Returns an error if no guest exports the HTTP handler, no route matches the
/// request path, the guest traps or returns an error, or the response cannot be
/// converted and collected.
pub async fn handle<B>(
    runtime: &Runtime<B>, request: http::Request<Bytes>,
) -> Result<http::Response<Bytes>>
where
    B: Clone + Send + Sync + 'static,
    StoreCtx<B>: WasiHttpView,
{
    HttpHarness::new(runtime.clone())?.handle(request).await
}

/// A request builder targeting `http://localhost{path}` with the `Host`
/// header every helper shares.
fn request(method: http::Method, path: &str) -> http::request::Builder {
    http::Request::builder()
        .method(method)
        .uri(format!("http://localhost{path}"))
        .header(http::header::HOST, "localhost")
}

fn get_request(path: &str) -> Result<http::Request<Bytes>> {
    request(http::Method::GET, path).body(Bytes::new()).context("building GET request")
}

fn post_request(path: &str, body: impl Into<Bytes>) -> Result<http::Request<Bytes>> {
    request(http::Method::POST, path).body(body.into()).context("building POST request")
}

fn delete_request(path: &str) -> Result<http::Request<Bytes>> {
    request(http::Method::DELETE, path).body(Bytes::new()).context("building DELETE request")
}

fn post_json_request(path: &str, body: impl Into<Bytes>) -> Result<http::Request<Bytes>> {
    request(http::Method::POST, path)
        .header(http::header::CONTENT_TYPE, "application/json")
        .body(body.into())
        .context("building POST request")
}

/// Drive a `GET {path}` request through the runtime's HTTP guest.
///
/// # Errors
///
/// Propagates any error from [`handle`], plus a request-construction error.
pub async fn get<B>(runtime: &Runtime<B>, path: &str) -> Result<http::Response<Bytes>>
where
    B: Clone + Send + Sync + 'static,
    StoreCtx<B>: WasiHttpView,
{
    handle(runtime, get_request(path)?).await
}

/// Drive a `POST {path}` request carrying `body` through the runtime's HTTP
/// guest.
///
/// # Errors
///
/// Propagates any error from [`handle`], plus a request-construction error.
pub async fn post<B>(
    runtime: &Runtime<B>, path: &str, body: impl Into<Bytes>,
) -> Result<http::Response<Bytes>>
where
    B: Clone + Send + Sync + 'static,
    StoreCtx<B>: WasiHttpView,
{
    handle(runtime, post_request(path, body)?).await
}

/// Drive a `DELETE {path}` request through the runtime's HTTP guest.
///
/// # Errors
///
/// Propagates any error from [`handle`], plus a request-construction error.
pub async fn delete<B>(runtime: &Runtime<B>, path: &str) -> Result<http::Response<Bytes>>
where
    B: Clone + Send + Sync + 'static,
    StoreCtx<B>: WasiHttpView,
{
    handle(runtime, delete_request(path)?).await
}

/// Drive a `POST {path}` request carrying a JSON `body`, tagged
/// `Content-Type: application/json` so axum's `Json` extractor accepts it.
///
/// # Errors
///
/// Propagates any error from [`handle`], plus a request-construction error.
pub async fn post_json<B>(
    runtime: &Runtime<B>, path: &str, body: impl Into<Bytes>,
) -> Result<http::Response<Bytes>>
where
    B: Clone + Send + Sync + 'static,
    StoreCtx<B>: WasiHttpView,
{
    handle(runtime, post_json_request(path, body)?).await
}
