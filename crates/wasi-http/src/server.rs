//! #HTTP Server

use std::clone::Clone;
use std::convert::Infallible;
use std::fmt::Debug;
use std::future::Future;
use std::pin::{Pin, pin};
use std::sync::Arc;
use std::task::{Context as TaskContext, Poll};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, anyhow, bail};
use bytes::Bytes;
use http::StatusCode;
use http::uri::{PathAndQuery, Uri};
use http_body_util::combinators::UnsyncBoxBody;
use http_body_util::{BodyExt, Full};
use hyper::body::{Body, Frame, SizeHint};
use hyper::header::{FORWARDED, HOST, HeaderValue, RETRY_AFTER};
use hyper::service::service_fn;
use hyper_util::rt::TokioExecutor;
use hyper_util::server::conn::auto;
use omnia_core::{
    Backends, ChainCtx, EnterError, FromEnv as _, HttpRoutes, Provides, Runtime, StoreCtx,
    TriggerRouter,
};
use tokio::net::TcpListener;
use tokio::sync::oneshot;
use tokio::task::{JoinHandle, JoinSet};
use tracing::{Instrument, info_span, instrument};
use wasmtime::{AsContextMut as _, Store};
use wasmtime_wasi_http::io::TokioIo;
use wasmtime_wasi_http::p3::bindings::http::types as wasi;
use wasmtime_wasi_http::p3::bindings::{Service, ServiceIndices};

use super::{WasiHttp, WasiHttpCtxView};

/// The streaming body of a response produced by [`HttpHandler::handle`].
pub type OutgoingBody = UnsyncBoxBody<Bytes, anyhow::Error>;

const HTTP_ADDR: &str = "0.0.0.0:8080";

// the header omnia-sdk's `Metadata` reads a guest's deadline from
const DEADLINE: &str = "x-deadline";

#[expect(missing_docs, reason = "`FromEnv` has no docs")]
mod options {
    use std::time::Duration;

    use fromenv::{FromEnv, ParseResult};

    /// Options for the HTTP trigger's server.
    ///
    /// [`HttpHandler::new`](super::HttpHandler::new) loads them from the
    /// environment, and [`HttpHandler::with_options`](super::HttpHandler::with_options)
    /// takes them in code.
    #[derive(Debug, Clone, FromEnv)]
    pub struct ServerOptions {
        /// The address the server listens on (`HTTP_ADDR`, default `0.0.0.0:8080`).
        #[env(from = "HTTP_ADDR", default = "0.0.0.0:8080")]
        pub addr: String,
        /// How long a request waits for a slot of the concurrency gate before it is refused with `503` (`HTTP_QUEUE_WAIT_MS`, default 0).
        #[env(from = "HTTP_QUEUE_WAIT_MS", default = "0", with = parse_millis)]
        pub queue_wait: Duration,
        /// The path a readiness probe requests (`HTTP_HEALTH_PATH`, unset for none).
        ///
        /// A request for it is answered `200` before it is routed. It enters
        /// no guest, so it takes no slot of the concurrency gate, and it wins
        /// over a guest route that covers it.
        #[env(from = "HTTP_HEALTH_PATH")]
        pub health_path: Option<String>,
    }

    fn parse_millis(value: &str) -> ParseResult<Duration> {
        Ok(Duration::from_millis(value.parse::<u64>()?))
    }
}

pub use options::ServerOptions;

impl omnia_core::FromEnv for ServerOptions {
    fn load_env() -> Result<Self> {
        Self::from_env().finalize().context("issue loading http server options")
    }
}

impl Default for ServerOptions {
    fn default() -> Self {
        Self {
            addr: HTTP_ADDR.to_owned(),
            queue_wait: Duration::ZERO,
            health_path: None,
        }
    }
}

#[instrument("http-server", skip(state))]
pub async fn run<B>(state: &Runtime<B>) -> Result<()>
where
    B: Provides<WasiHttp>,
{
    let Some(handler) = HttpHandler::new(state)? else {
        tracing::info!("no guest exports the http handler; http trigger inert");
        return Ok(());
    };

    let addr = &handler.options.addr;
    let listener = TcpListener::bind(addr).await.with_context(|| format!("binding {addr}"))?;
    let addr = listener.local_addr().context("reading http listener address")?;
    tracing::info!("{} http server listening on: {addr}", handler.component);

    // built once, cloned per connection
    let builder = auto::Builder::new(TokioExecutor::new());
    let mut connections = JoinSet::new();

    // listen for requests until terminated
    loop {
        let (stream, _) = tokio::select! {
            () = state.stopped() => break,
            Some(_) = connections.join_next(), if !connections.is_empty() => continue,
            accepted = listener.accept() => match accepted {
                Ok(conn) => conn,
                Err(error) => {
                    // a transient accept error must not tear down the server
                    tracing::error!(%error, "accept error");
                    continue;
                }
            },
        };
        if let Err(error) = stream.set_nodelay(true) {
            tracing::warn!(%error, "failed to set TCP_NODELAY");
        }
        let stream = TokioIo::new(stream);
        let handler = handler.clone();
        let builder = builder.clone();
        let state = state.clone();

        connections.spawn(async move {
            let service = service_fn(move |request| {
                let handler = handler.clone();
                async move {
                    let response = handler.handle(request).await.unwrap_or_else(|e| {
                        tracing::error!("Error proxying request: {e}");
                        internal_error()
                    });

                    // track server error responses; a refusal at capacity is
                    // load, not a processing error, and must not log per request
                    if response.status() >= StatusCode::INTERNAL_SERVER_ERROR
                        && response.status() != StatusCode::SERVICE_UNAVAILABLE
                    {
                        tracing::error!(
                            monotonic_counter.processing_errors = 1,
                            service = %handler.component,
                            error = format!("{response:?}"),
                        );
                    }
                    Ok::<_, Infallible>(response)
                }
            });
            let conn = builder.serve_connection_with_upgrades(stream, service);
            drive_connection(conn, &state, auto::UpgradeableConnection::graceful_shutdown).await;
        });
    }

    drop(listener);
    while connections.join_next().await.is_some() {}
    Ok(())
}

async fn drive_connection<B, F, G, E>(conn: F, state: &Runtime<B>, graceful: G)
where
    B: Backends,
    F: Future<Output = std::result::Result<(), E>>,
    E: Debug,
    G: FnOnce(Pin<&mut F>),
{
    let mut conn = pin!(conn);
    let result = tokio::select! {
        result = conn.as_mut() => result,
        () = state.stopped() => {
            graceful(conn.as_mut());
            conn.await
        }
    };
    if let Err(e) = result {
        tracing::error!("connection error: {e:?}");
    }
}

/// In-process `wasi:http/handler` dispatcher: routes a request to a
/// guest, instantiates it, and invokes the export.
///
/// The server loop drives one over accepted connections; tests drive it
/// directly with a constructed request, bypassing the socket.
#[derive(Clone)]
pub struct HttpHandler<B>
where
    B: Provides<WasiHttp>,
{
    state: Runtime<B>,
    component: Arc<str>,
    routing: Arc<TriggerRouter<HttpRoutes>>,
    options: Arc<ServerOptions>,
}

impl<B> HttpHandler<B>
where
    B: Provides<WasiHttp>,
{
    /// Build the handler over `runtime`'s registry under the
    /// [`ServerOptions`] the environment sets; `None` when no guest exports
    /// the http handler (the trigger is inert).
    ///
    /// # Errors
    ///
    /// Returns an error if:
    ///
    /// - the server options do not parse;
    /// - the [health path](ServerOptions::health_path) does not start with `/`;
    /// - the deployment's http routes are inconsistent with the guests' exports.
    pub fn new(runtime: &Runtime<B>) -> Result<Option<Self>> {
        Self::with_options(runtime, ServerOptions::load_env()?)
    }

    /// Build the handler over `runtime`'s registry under `options`; `None`
    /// when no guest exports the http handler (the trigger is inert).
    ///
    /// # Errors
    ///
    /// Returns an error if the [health path](ServerOptions::health_path) does
    /// not start with `/`, or the deployment's http routes are inconsistent
    /// with the guests' exports.
    pub fn with_options(runtime: &Runtime<B>, options: ServerOptions) -> Result<Option<Self>> {
        if let Some(path) = &options.health_path
            && !path.starts_with('/')
        {
            bail!("HTTP_HEALTH_PATH `{path}` must start with `/`");
        }

        // a guest is capable exactly when its `ServiceIndices` resolve; a
        // declared guest is probed when its first request loads it
        let routing = runtime.http_trigger_router(ServiceIndices::new)?;
        if routing.is_inert() {
            return Ok(None);
        }
        Ok(Some(Self {
            state: runtime.clone(),
            component: Arc::from(runtime.name()),
            routing: Arc::new(routing),
            options: Arc::new(options),
        }))
    }

    /// Forward a request to the routed guest and return its response.
    ///
    /// The request is normalised first (scheme and authority from `Forwarded`,
    /// the URI, or `Host`). Once the guest holds a slot of the runtime's
    /// concurrency gate, the request's `x-deadline` header is set to the
    /// instant the guest timeout expires, in milliseconds since the Unix
    /// epoch, replacing any the client sent.
    ///
    /// Outcomes the handler can answer itself come back as responses:
    ///
    /// - `200` for the [health path](ServerOptions::health_path), at once;
    /// - `400` for a request with no authority;
    /// - `404` for an unrouted path;
    /// - `500` for a guest that times out, cannot be loaded at its first use,
    ///   or is no longer registered;
    /// - `503` with `Retry-After` when the runtime is at capacity: no slot
    ///   freed within [`queue_wait`](ServerOptions::queue_wait), or the
    ///   instance pool is full.
    ///
    /// # Errors
    ///
    /// Returns an error if the guest cannot be instantiated, traps, returns an
    /// error, or yields a response that cannot be converted. In-process
    /// callers see these as `Err`; only the server loop maps them to `500`.
    pub async fn handle<T>(&self, request: http::Request<T>) -> Result<http::Response<OutgoingBody>>
    where
        T: Body<Data = Bytes> + Send + 'static,
        T::Error: Into<wasmtime_wasi_http::Error>,
    {
        tracing::debug!(method = %request.method(), uri = %request.uri(), "handling request");

        // a readiness probe is answered before any guest, so even at capacity
        if self.options.health_path.as_deref() == Some(request.uri().path()) {
            return Ok(healthy());
        }

        // normalise the request; a bad `Host` header is a 400, not a 500
        let mut request = match fix_request(request) {
            Ok(request) => request,
            Err(error) => {
                tracing::debug!(%error, "rejecting malformed request");
                return Ok(bad_request());
            }
        };

        // resolve the guest by path; unmatched is 404
        let Some(guest_id) = self.routing.resolve(request.uri().path()) else {
            return Ok(not_found());
        };

        // a routed guest that fails to load is a 500, never a panic
        let guest = match self.state.guest(guest_id).await {
            Ok(guest) => guest,
            Err(error) => {
                tracing::error!(guest = %guest_id, %error, "routed guest unavailable; returning 500");
                return Ok(internal_error());
            }
        };

        // a declared guest's export is checked here, at first use
        let indices =
            ServiceIndices::new(guest.instance_pre()).map_err(anyhow::Error::from).with_context(
                || format!("routed guest `{guest_id}` does not export `wasi:http/handler`"),
            )?;

        // instantiate the selected guest fresh (instance-per-call) on a gate
        // slot; a runtime at capacity is a 503 the client may retry
        let wait = Some(self.options.queue_wait);
        let (mut store, instance) = match self.state.enter(&guest, ChainCtx::server(), wait).await {
            Ok(entered) => entered,
            Err(EnterError::Busy) => {
                tracing::debug!(guest = %guest_id, "runtime at capacity; returning 503");
                return Ok(unavailable());
            }
            Err(error) => return Err(error.into()),
        };
        let service = indices.load(&mut store, &instance)?;

        // the guest's clock starts once it holds a slot
        let timeout = self.state.options().guest_timeout;
        request.headers_mut().insert(DEADLINE, HeaderValue::from(deadline_millis(timeout)));

        let (sender, receiver) = oneshot::channel::<Result<hyper::Response<OutgoingBody>>>();
        let guest_task = spawn_guest(store, service, request, sender);

        // bound time-to-response (not the streaming body); cancel a hung guest
        let response = match tokio::time::timeout(timeout, receiver).await {
            Ok(delivered) => {
                delivered.map_err(|_canceled| anyhow!("guest produced no response"))??
            }
            Err(_elapsed) => {
                guest_task.abort();
                tracing::error!(service = %self.component, "guest handler timed out");
                return Ok(internal_error());
            }
        };
        tracing::debug!("received response: {response:?}");

        Ok(response)
    }
}

// Runs the guest's handler on a task of its own, which owns the store and so
// the invocation's gate slot. The response or the failure goes through
// `sender`, and the task lives on until hyper has consumed the body.
fn spawn_guest<B, T>(
    mut store: Store<StoreCtx<B>>, service: Service, request: http::Request<T>,
    sender: oneshot::Sender<Result<hyper::Response<OutgoingBody>>>,
) -> JoinHandle<()>
where
    B: Provides<WasiHttp>,
    T: Body<Data = Bytes> + Send + 'static,
    T::Error: Into<wasmtime_wasi_http::Error>,
{
    tokio::spawn(async move {
        let result = store
            .run_concurrent(async |store| {
                // one error path through `sender`, so the caller never
                // mistakes a real error for a panicked task
                let built = async move {
                    let (request, io) = store.with(|mut access| {
                        let mut cx = access.as_context_mut();
                        wasi::Request::from_http(view(cx.data_mut()).hooks, request)
                    });
                    let req = store
                        .with(|mut store| {
                            view(store.as_context_mut().data_mut()).table.push(request)
                        })
                        .context("failed to push request to table")?;

                    let res = service
                        .wasi_http_handler()
                        .call_handle(store, req)
                        .await
                        .map_err(anyhow::Error::from)
                        .context("guest trap")?
                        .map_err(|e| anyhow!("guest error: {e}"))?;
                    let wasi_resp = store
                        .with(|mut store| view(store.as_context_mut().data_mut()).table.delete(res))
                        .context("failed to delete response from table")?;
                    store
                        .with(|mut store| wasi_resp.into_http_with_getter(&mut store, io, view))
                        .map_err(|e| anyhow!("converting guest response: {e}"))
                }
                .await;

                match built {
                    Ok(resp) => {
                        // keep `run_concurrent` alive until hyper has consumed
                        // the body, so the wasi pipe resources stay valid
                        let (body_done_tx, body_done_rx) = oneshot::channel::<()>();
                        let resp = resp.map(|body| {
                            BodyDoneWrapper {
                                body: body.map_err(Into::into),
                                _tx: body_done_tx,
                            }
                            .boxed_unsync()
                        });
                        if sender.send(Ok(resp)).is_ok() {
                            _ = body_done_rx.await;
                        }
                    }
                    Err(error) => {
                        _ = sender.send(Err(error));
                    }
                }

                anyhow::Ok(())
            })
            .instrument(info_span!("http-request"))
            .await;

        match result {
            Ok(Ok(())) => {}
            Ok(Err(e)) => tracing::error!("http guest task error: {e:#}"),
            Err(e) => tracing::error!("run_concurrent error: {e:?}"),
        }
    })
}

// The sender drops once the body is consumed (or the wrapper dropped); its
// receiver keeps `run_concurrent` alive while hyper streams the response.
struct BodyDoneWrapper<B> {
    body: B,
    _tx: oneshot::Sender<()>,
}

impl<B> Body for BodyDoneWrapper<B>
where
    B: Body + Unpin,
{
    type Data = B::Data;
    type Error = B::Error;

    fn poll_frame(
        self: Pin<&mut Self>, cx: &mut TaskContext<'_>,
    ) -> Poll<Option<Result<Frame<Self::Data>, Self::Error>>> {
        let inner = Pin::new(&mut self.get_mut().body);
        inner.poll_frame(cx)
    }

    fn size_hint(&self) -> SizeHint {
        self.body.size_hint()
    }

    fn is_end_stream(&self) -> bool {
        self.body.is_end_stream()
    }
}

// rebuild the uri with scheme and authority set, so they reach the guest
fn fix_request<T>(request: http::Request<T>) -> Result<http::Request<T>> {
    let p_and_q =
        request.uri().path_and_query().map_or_else(|| PathAndQuery::from_static("/"), Clone::clone);
    let mut uri_builder = Uri::builder().path_and_query(p_and_q);

    if let Some(forwarded) = request.headers().get(FORWARDED) {
        // behind a proxy: the first RFC 7239 element is the client-facing hop
        let element = forwarded.to_str()?.split(',').next().unwrap_or_default();
        let mut scheme = "http";
        for parameter in element.split(';') {
            let Some((name, value)) = parameter.split_once('=') else {
                continue;
            };
            let value = value.trim().trim_matches('"');
            match name.trim().to_ascii_lowercase().as_str() {
                "host" => uri_builder = uri_builder.authority(value),
                "proto" => {
                    scheme = if value.eq_ignore_ascii_case("https") { "https" } else { "http" }
                }
                _ => {}
            }
        }
        uri_builder = uri_builder.scheme(scheme);
    } else {
        // running locally: authority from the URI (HTTP/2 `:authority` or
        // absolute-form HTTP/1.1), falling back to the `Host` header
        let scheme = request.uri().scheme_str().unwrap_or("http");
        let authority = match request.uri().authority() {
            Some(authority) => authority.as_str(),
            None => match request.headers().get(HOST) {
                Some(host) => host.to_str()?,
                None => return Err(anyhow!("missing authority in URI and host header")),
            },
        };
        uri_builder = uri_builder.authority(authority).scheme(scheme);
    }

    let (mut parts, body) = request.into_parts();
    parts.uri = uri_builder.build()?;
    let request = http::Request::from_parts(parts, body);

    Ok(request)
}

// saturates rather than panics, so no configured timeout can fail a request
fn deadline_millis(timeout: Duration) -> u64 {
    let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default();
    u64::try_from(now.saturating_add(timeout).as_millis()).unwrap_or(u64::MAX)
}

fn healthy() -> hyper::Response<OutgoingBody> {
    let body = Full::new(Bytes::from_static(b"OK")).map_err(Into::into).boxed_unsync();

    hyper::Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", "text/plain; charset=UTF-8")
        .body(body)
        .expect("should build health response")
}

fn bad_request() -> hyper::Response<OutgoingBody> {
    let body = Full::new(Bytes::from_static(b"Bad Request")).map_err(Into::into).boxed_unsync();

    hyper::Response::builder()
        .status(StatusCode::BAD_REQUEST)
        .header("Content-Type", "text/plain; charset=UTF-8")
        .body(body)
        .expect("should build bad request response")
}

fn not_found() -> hyper::Response<OutgoingBody> {
    let body = Full::new(Bytes::from_static(b"Not Found")).map_err(Into::into).boxed_unsync();

    hyper::Response::builder()
        .status(StatusCode::NOT_FOUND)
        .header("Content-Type", "text/plain; charset=UTF-8")
        .body(body)
        .expect("should build not found response")
}

fn unavailable() -> hyper::Response<OutgoingBody> {
    let body =
        Full::new(Bytes::from_static(b"Service Unavailable")).map_err(Into::into).boxed_unsync();

    hyper::Response::builder()
        .status(StatusCode::SERVICE_UNAVAILABLE)
        .header("Content-Type", "text/plain; charset=UTF-8")
        .header(RETRY_AFTER, "1")
        .body(body)
        .expect("should build service unavailable response")
}

const BODY: &str = r"<!doctype html>
<html>
<head>
    <title>500 Internal Server Error</title>
</head>
<body>
    <center>
        <h1>500 Internal Server Error</h1>
        <hr>
        <pre>Guest error</pre>
    </center>
</body>
</html>";

fn internal_error() -> hyper::Response<OutgoingBody> {
    let body = Full::new(Bytes::from(BODY)).map_err(Into::into).boxed_unsync();

    hyper::Response::builder()
        .status(StatusCode::INTERNAL_SERVER_ERROR)
        .header("Content-Type", "text/html; charset=UTF-8")
        .body(body)
        .expect("should build internal error response")
}

fn view<B: Provides<WasiHttp>>(store: &mut StoreCtx<B>) -> WasiHttpCtxView<'_> {
    store.view::<WasiHttp>()
}
