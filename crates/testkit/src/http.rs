//! Driving a guest's `wasi:http/handler` export in-process.
//!
//! [`HttpHarness`] mirrors the runtime's HTTP trigger server
//! (`crates/wasi-http/src/host/server.rs`) — snapshot the trigger router once
//! at construction (the analogue of the server's boot), then per request
//! resolve the guest by path (static route first, deployment fallback second),
//! instantiate it fresh, hand it a `wasi:http` request, and convert the
//! response back — but skips the TCP socket and collects the response body
//! eagerly so a test can assert on it directly. The free helpers ([`handle`],
//! [`get`], [`post`], [`delete`], [`post_json`]) wrap a single-use harness;
//! a scenario spanning several requests (e.g. dynamic registration through
//! the fallback) must drive one harness so routing keeps the production
//! server's boot-frozen lifetime.

use std::sync::Arc;

use anyhow::{Context as _, Result, anyhow, ensure};
use bytes::Bytes;
use http_body_util::{BodyExt as _, Full};
use omnia::{Guest, HttpRoutes, Runtime, StoreCtx, TriggerRouter};
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
    /// the HTTP handler and no fallback is installed (mirroring the server's
    /// inert check).
    pub fn new(runtime: Runtime<B>) -> Result<Self> {
        let routing = TriggerRouter::build(
            runtime.registry(),
            "http",
            runtime.registry().routes().http().clone(),
            ServiceIndices::new,
        )?;
        ensure!(
            !routing.is_inert() || runtime.http_fallback().is_some(),
            "no guest exports the http handler"
        );
        Ok(Self { runtime, routing })
    }

    /// Drive one request through the two-tier path (static route, then
    /// deployment fallback) and return its fully-collected response.
    ///
    /// # Errors
    ///
    /// Returns an error if no route or fallback matches the request path, the
    /// fallback guest cannot be resolved or lacks the handler, the guest
    /// traps or returns an error, or the response cannot be converted and
    /// collected.
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
            // Static miss: fallback → ensure (resolve-on-miss) →
            // request-local indices, as the production server does.
            let target = self
                .runtime
                .http_fallback()
                .and_then(|fallback| fallback(&path))
                .with_context(|| format!("no route matched path `{path}`"))?;
            guest = self
                .runtime
                .ensure_guest(&target, "wasi:http/handler")
                .await
                .with_context(|| format!("resolving fallback guest for path `{path}`"))?;
            late_indices =
                ServiceIndices::new(guest.instance_pre())
                    .map_err(anyhow::Error::from)
                    .with_context(|| format!("fallback guest `{target}` lacks the http handler"))?;
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

fn get_request(path: &str) -> Result<http::Request<Bytes>> {
    http::Request::get(format!("http://localhost{path}"))
        .header(http::header::HOST, "localhost")
        .body(Bytes::new())
        .context("building GET request")
}

fn post_request(path: &str, body: impl Into<Bytes>) -> Result<http::Request<Bytes>> {
    http::Request::post(format!("http://localhost{path}"))
        .header(http::header::HOST, "localhost")
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
    let request = http::Request::delete(format!("http://localhost{path}"))
        .header(http::header::HOST, "localhost")
        .body(Bytes::new())
        .context("building DELETE request")?;
    handle(runtime, request).await
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
    let request = http::Request::post(format!("http://localhost{path}"))
        .header(http::header::HOST, "localhost")
        .header(http::header::CONTENT_TYPE, "application/json")
        .body(body.into())
        .context("building POST request")?;
    handle(runtime, request).await
}
