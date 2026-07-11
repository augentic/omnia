//! Driving a guest's `wasi:http/handler` export in-process.
//!
//! [`handle`] mirrors the runtime's HTTP trigger server
//! (`crates/wasi-http/src/host/server.rs`) — resolve the guest by request path,
//! instantiate it fresh, hand it a `wasi:http` request, and convert the
//! response back — but skips the TCP socket and collects the response body
//! eagerly so a test can assert on it directly. [`get`] and [`post`] are thin
//! builders over it.

use anyhow::{Context as _, Result, anyhow, ensure};
use bytes::Bytes;
use http_body_util::{BodyExt as _, Full};
use omnia::{Runtime, StoreCtx, TriggerRouter};
use wasmtime_wasi_http::p3::WasiHttpView;
use wasmtime_wasi_http::p3::bindings::ServiceIndices;
use wasmtime_wasi_http::p3::bindings::http::types::{self as wasi};

/// Drive one request through the runtime's sole `wasi:http` guest and return
/// its fully-collected response.
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
    // Capability + route resolution, exactly as the HTTP server does at startup.
    let routing = TriggerRouter::build(
        runtime.registry(),
        "http",
        runtime.registry().routes().http().clone(),
        ServiceIndices::new,
    )?;
    ensure!(!routing.is_inert(), "no guest exports the http handler");

    let path = request.uri().path().to_owned();
    let (guest_id, indices) =
        routing.resolve(&path).with_context(|| format!("no route matched path `{path}`"))?;
    let guest = runtime.registry().get(guest_id).context("resolved guest is registered")?;

    // Instantiate fresh (instance-per-call) and load the typed handler.
    let mut store = runtime.build_store(runtime.store());
    let instance = runtime.instantiate(guest.instance_pre(), &mut store).await?;
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
    let request = http::Request::get(format!("http://localhost{path}"))
        .header(http::header::HOST, "localhost")
        .body(Bytes::new())
        .context("building GET request")?;
    handle(runtime, request).await
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
    let request = http::Request::post(format!("http://localhost{path}"))
        .header(http::header::HOST, "localhost")
        .body(body.into())
        .context("building POST request")?;
    handle(runtime, request).await
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
