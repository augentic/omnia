//! #HTTP Server

use std::clone::Clone;
use std::convert::Infallible;
use std::fmt::Debug;
use std::future::Future;
use std::pin::{Pin, pin};
use std::sync::Arc;
use std::task::{Context as TaskContext, Poll};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, anyhow, bail, ensure};
use bytes::Bytes;
use http::uri::{Authority, PathAndQuery, Uri};
use http::{StatusCode, Version};
use http_body_util::combinators::UnsyncBoxBody;
use http_body_util::{BodyExt, Full};
use hyper::body::{Body, Frame, SizeHint};
use hyper::header::{FORWARDED, HOST, HeaderValue, RETRY_AFTER};
use hyper::service::service_fn;
use hyper_util::rt::TokioExecutor;
use hyper_util::server::conn::auto;
use omnia_core::{
    Backends, ChainCtx, Entered, FromEnv, HttpRoutes, Provides, Runtime, StoreCtx, TriggerRouter,
    telemetry,
};
use tokio::net::TcpListener;
use tokio::sync::oneshot;
use tokio::task::JoinSet;
use tokio::time::timeout;
use tracing::{Instrument, info_span, instrument};
use wasmtime::AsContextMut as _;
use wasmtime_wasi_http::io::TokioIo;
use wasmtime_wasi_http::p3::bindings::http::types as wasi;
use wasmtime_wasi_http::p3::bindings::{Service, ServiceIndices};

use super::{WasiHttp, WasiHttpCtxView};

/// The streaming body of a response produced by [`HttpHandler::handle`].
pub type OutgoingBody = UnsyncBoxBody<Bytes, anyhow::Error>;

// the header omnia-sdk's `Metadata` reads a guest's deadline from
const DEADLINE: &str = "x-deadline";

/// Options for the HTTP trigger's server.
///
/// [`HttpHandler::new`] loads them from the environment, and
/// [`HttpHandler::with_options`] takes them in code.
#[derive(Clone, Debug, FromEnv)]
#[env(validate = Self::validate)]
pub struct ServerOptions {
    /// The address the server listens on (`HTTP_ADDR`, default `0.0.0.0:8080`).
    #[env(from = "HTTP_ADDR", default = "0.0.0.0:8080")]
    pub addr: String,
    /// The path a probe requests, answered `200` without entering a guest (`HTTP_HEALTH_PATH`, unset: none).
    ///
    /// The path is matched exactly, before the request's target is checked
    /// or it is routed. It wins over a guest route that covers it, and a path
    /// below it goes to the guest as usual.
    #[env(from = "HTTP_HEALTH_PATH")]
    pub health_path: Option<String>,
}

impl ServerOptions {
    /// Checks that the HTTP trigger can serve under these options.
    ///
    /// [`load_env`](Self::load_env), [`with_env`](Self::with_env), and
    /// [`load_from`](Self::load_from) run it on what they load, and
    /// [`HttpHandler::with_options`] runs it on the options it is given.
    ///
    /// # Errors
    ///
    /// Returns an error when the [health path](Self::health_path) is not a
    /// request path, one that starts with `/` and holds no `?`, `#`, or
    /// whitespace.
    pub fn validate(&self) -> Result<()> {
        if let Some(path) = &self.health_path {
            let plain = !path.contains(|c: char| c == '?' || c == '#' || c.is_whitespace());
            ensure!(
                path.starts_with('/') && plain,
                "HTTP_HEALTH_PATH must start with `/` and hold no `?`, `#`, or whitespace"
            );
        }
        Ok(())
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

                    // track server error responses
                    if response.status() >= StatusCode::INTERNAL_SERVER_ERROR {
                        telemetry::count_error(&handler.component, None);
                        tracing::error!(
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
    /// Returns an error if the server options do not load or
    /// [validate](ServerOptions::validate), or the deployment's http routes
    /// are inconsistent with the guests' exports.
    pub fn new(runtime: &Runtime<B>) -> Result<Option<Self>> {
        Self::with_options(runtime, ServerOptions::load_env()?)
    }

    /// Build the handler over `runtime`'s registry under `options`; `None`
    /// when no guest exports the http handler (the trigger is inert).
    ///
    /// An inert trigger answers no [health path](ServerOptions::health_path),
    /// so it logs a warning when `options` sets one.
    ///
    /// # Errors
    ///
    /// Returns an error if `options` do not [validate](ServerOptions::validate),
    /// or the deployment's http routes are inconsistent with the guests'
    /// exports.
    pub fn with_options(runtime: &Runtime<B>, options: ServerOptions) -> Result<Option<Self>> {
        options.validate()?;

        // a guest is capable exactly when its `ServiceIndices` resolve; a
        // declared guest is probed when its first request loads it
        let routing = runtime.http_trigger_router(ServiceIndices::new)?;
        if routing.is_inert() {
            if let Some(path) = &options.health_path {
                tracing::warn!(%path, "HTTP_HEALTH_PATH is set, but no guest serves http to answer it");
            }
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
    /// A request for the [health path](ServerOptions::health_path) is answered
    /// at once, before its target is checked, so a probe needs no `Host` and
    /// takes no slot of the runtime's concurrency gate. Any other request is
    /// normalised first: its scheme and authority come from `Forwarded` where
    /// it names them, else from the URI or `Host`. Its `x-deadline` header is
    /// set to the instant the guest timeout expires, in milliseconds since the
    /// Unix epoch, replacing any the client sent.
    ///
    /// Outcomes the handler can answer itself come back as responses:
    /// - `200` for the health path;
    /// - `400` for a malformed request target;
    /// - `404` for an unrouted path;
    /// - `500` for a guest that times out, cannot be loaded at its first use,
    ///   or is no longer registered;
    /// - `503`, with `Retry-After: 1`, when every slot of the runtime's
    ///   [concurrency gate](Runtime::try_slot) is held.
    ///
    /// A request target is malformed when the request:
    /// - carries no `Host`, unless it is HTTP/2 with an `:authority`;
    /// - carries more than one `Host`, or one that does not parse;
    /// - names an authority with userinfo or a non-numeric port;
    /// - is HTTP/2 with an `:authority` other than its `Host`.
    ///
    /// The [guest timeout](omnia_core::RuntimeOptions::guest_timeout) counts
    /// from when the request takes its slot, so it bounds instantiation as well
    /// as the wait for the response head. Dropping the future before it
    /// resolves stops the guest and frees its slot.
    ///
    /// The response body streams on after `handle` returns. It fails part way
    /// when the guest traps, or once
    /// [`http_stream_timeout`](omnia_core::RuntimeOptions::http_stream_timeout)
    /// has passed since the head was sent.
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

        // answer a probe before the request is checked, so it needs no `Host`
        if self.options.health_path.as_deref() == Some(request.uri().path()) {
            return Ok(healthy());
        }

        // normalise the request and stamp the guest's deadline; a bad `Host`
        // header is a 400, not a 500
        let request = match fix_request(request, self.state.options().guest_timeout) {
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

        // a full gate is a 503, answered before a store is built
        let Some(slot) = self.state.try_slot() else {
            return Ok(unavailable());
        };

        // dropping the set aborts the guest's task, so a request dropped or
        // timed out before its head frees its slot
        let mut guest_task = JoinSet::new();

        // instantiate the selected guest fresh (instance-per-call), bounding
        // the time from the slot to the head (not the streaming body)
        let invoked = timeout(self.state.options().guest_timeout, async {
            let (mut entered, instance) =
                self.state.enter(&guest, ChainCtx::server(), slot).await?;
            let service = indices.load(&mut entered, &instance)?;

            let (sender, receiver) = oneshot::channel::<Result<hyper::Response<OutgoingBody>>>();
            let stream_timeout = self.state.options().http_stream_timeout;
            spawn_guest(&mut guest_task, entered, service, request, sender, stream_timeout);
            receiver.await.map_err(|_canceled| anyhow!("guest produced no response"))?
        })
        .await;
        let response = match invoked {
            Ok(response) => response?,
            Err(_elapsed) => {
                tracing::error!(service = %self.component, "guest handler timed out");
                return Ok(internal_error());
            }
        };

        // the body's drop ends the task from here on
        guest_task.detach_all();
        tracing::debug!("received response: {response:?}");

        Ok(response)
    }
}

// Runs the guest's handler on a task of its own in `tasks`, which owns the
// store and the request's slot of the gate. The response or the failure goes
// through `sender`, and the task lives on until hyper has consumed the body or
// `stream_timeout` has passed since the head went out. Ending the task drops
// the store, which fails a body still streaming, and then frees the slot.
fn spawn_guest<B, T>(
    tasks: &mut JoinSet<()>, mut entered: Entered<B>, service: Service, request: http::Request<T>,
    sender: oneshot::Sender<Result<hyper::Response<OutgoingBody>>>,
    stream_timeout: Option<Duration>,
) where
    B: Provides<WasiHttp>,
    T: Body<Data = Bytes> + Send + 'static,
    T::Error: Into<wasmtime_wasi_http::Error>,
{
    tasks.spawn(async move {
        let result = entered
            .as_context_mut()
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
                            let consumed = async { _ = body_done_rx.await };
                            match stream_timeout {
                                Some(limit) => {
                                    if timeout(limit, consumed).await.is_err() {
                                        tracing::warn!(
                                            ?limit,
                                            "response body outlived HTTP_STREAM_TIMEOUT_MS; cutting it"
                                        );
                                    }
                                }
                                None => consumed.await,
                            }
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
    });
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

// rebuild the uri with scheme and authority set, so they reach the guest, and
// stamp the guest's deadline over any the client sent
fn fix_request<T>(request: http::Request<T>, timeout: Duration) -> Result<http::Request<T>> {
    let mut authority = request_target(&request)?;
    let mut scheme = request.uri().scheme_str().unwrap_or("http");

    // behind a proxy: the first RFC 7239 element is the client-facing hop
    if let Some(forwarded) = request.headers().get(FORWARDED) {
        let element = forwarded.to_str()?.split(',').next().unwrap_or_default();
        scheme = "http";
        for parameter in element.split(';') {
            let Some((name, value)) = parameter.split_once('=') else {
                continue;
            };
            let value = value.trim().trim_matches('"');
            match name.trim().to_ascii_lowercase().as_str() {
                "host" => authority = checked(value.parse()?)?,
                "proto" if value.eq_ignore_ascii_case("https") => scheme = "https",
                _ => {}
            }
        }
    }

    let p_and_q =
        request.uri().path_and_query().map_or_else(|| PathAndQuery::from_static("/"), Clone::clone);
    let uri = Uri::builder().scheme(scheme).authority(authority).path_and_query(p_and_q).build()?;
    let (mut parts, body) = request.into_parts();
    parts.uri = uri;
    parts.headers.insert(DEADLINE, HeaderValue::from(deadline_millis(timeout)));

    Ok(http::Request::from_parts(parts, body))
}

// `Host` is required and single (RFC 9112 §3.2). HTTP/2 may carry
// `:authority` instead, and the two must agree when it sends both (RFC 9113
// §8.3.1). An HTTP/1.1 absolute-form target outranks `Host` (RFC 9112 §3.2.2).
fn request_target<T>(request: &http::Request<T>) -> Result<Authority> {
    let mut hosts = request.headers().get_all(HOST).iter();
    let host = hosts.next();
    ensure!(hosts.next().is_none(), "more than one `Host` header");
    let host = host.map(|host| checked(host.to_str()?.parse()?)).transpose()?;

    let target = request.uri().authority().cloned().map(checked).transpose()?;
    let http2 = request.version() == Version::HTTP_2;
    match (target, host) {
        (Some(target), Some(host)) => {
            ensure!(
                !http2 || target.as_str().eq_ignore_ascii_case(host.as_str()),
                "`:authority` {target} differs from `Host` {host}"
            );
            Ok(target)
        }
        (Some(target), None) if http2 => Ok(target),
        (None, Some(host)) => Ok(host),
        _ => bail!("no `Host` header"),
    }
}

// a request authority is `uri-host [ ":" port ]` (RFC 9110 §7.2), and userinfo
// in one is an error (§4.2.4); `Authority` parses userinfo and reads a
// malformed port as no port, so both are checked here
fn checked(authority: Authority) -> Result<Authority> {
    let text = authority.as_str();
    ensure!(!text.contains('@'), "authority {text} carries userinfo");

    let host = authority.host();
    ensure!(!host.is_empty(), "authority {text} names no host");
    let suffix = text.strip_prefix(host).unwrap_or(text);
    ensure!(
        suffix.is_empty()
            || suffix.strip_prefix(':').is_some_and(|port| {
                port.bytes().all(|b| b.is_ascii_digit()) && port.parse::<u16>().is_ok()
            }),
        "authority {text} has a malformed port"
    );

    Ok(authority)
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
