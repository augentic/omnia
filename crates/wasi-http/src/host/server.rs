//! #HTTP Server

use std::clone::Clone;
use std::convert::Infallible;
use std::env;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context as TaskContext, Poll};

use anyhow::{Context, Result, anyhow, bail};
use bytes::Bytes;
use http::StatusCode;
use http::uri::{PathAndQuery, Uri};
use http_body_util::combinators::UnsyncBoxBody;
use http_body_util::{BodyExt, Full};
use hyper::body::{Body, Frame, Incoming, SizeHint};
use hyper::header::{FORWARDED, HOST};
use hyper::server::conn::http1;
use hyper::service::service_fn;
use omnia::{Guest, HttpRoutes, RouteRefusal, RoutingPolicy, Runtime, StoreCtx, TriggerRouter};
use tokio::net::TcpListener;
use tokio::sync::oneshot;
use tokio::time::timeout;
use tracing::{Instrument, debug_span, instrument};
use wasmtime_wasi_http::io::TokioIo;
use wasmtime_wasi_http::p3::WasiHttpView;
use wasmtime_wasi_http::p3::bindings::ServiceIndices;
use wasmtime_wasi_http::p3::bindings::http::types::{self as wasi, ErrorCode};

type OutgoingBody = UnsyncBoxBody<Bytes, anyhow::Error>;

const HTTP_ADDR: &str = "0.0.0.0:8080";

#[instrument("http-server", skip(state))]
pub async fn run<B>(state: &Runtime<B>) -> Result<()>
where
    B: Clone + Send + Sync + 'static,
    StoreCtx<B>: WasiHttpView,
{
    let component = state.name().to_owned();

    // Adopt the deployment-supplied listener up front, before the inert
    // check can return: a pre-bound socket must be served or refused loudly,
    // never parked on the runtime.
    let supplied = state.take_http_listener();

    // Capability probe: a guest exports `wasi:http/incoming-handler` exactly
    // when its typed `ServiceIndices` resolve. The runtime builds the
    // per-guest indices and the router that selects among them once, up
    // front, under its deployment's routing policy (an installed `http_paths`
    // hook owns HTTP routing outright, so routing is table-driven only).
    let routing = state.http_trigger_router(ServiceIndices::new)?;
    // A hook-equipped deployment (`TableOnly`) may start inert and fault
    // guests in per request; only a deployment with neither routes nor hook
    // stays quiet.
    if routing.is_inert() && state.http_routing_policy() == RoutingPolicy::CapabilityDefault {
        if supplied.is_some() {
            bail!(
                "a pre-bound http listener was supplied but no guest exports the http handler; \
                 refusing to silently drop the socket"
            );
        }
        tracing::info!("no guest exports the http handler; http trigger inert");
        return Ok(());
    }

    // A deployment-supplied listener is authoritative (its local address is
    // already the guest-visible `HTTP_ADDR`); without one, bind from the
    // `HTTP_ADDR` environment variable.
    let listener = if let Some(listener) = supplied {
        listener.set_nonblocking(true).context("configuring supplied http listener")?;
        TcpListener::from_std(listener).context("adopting supplied http listener")?
    } else {
        let addr = env::var("HTTP_ADDR").unwrap_or_else(|_| HTTP_ADDR.into());
        TcpListener::bind(&addr).await.with_context(|| format!("binding {addr}"))?
    };
    let addr = listener.local_addr().context("reading http listener address")?;
    tracing::info!("{component} http server listening on: {addr}");

    let handler = Handler {
        state: state.clone(),
        component: Arc::from(component),
        routing: Arc::new(routing),
    };

    // `keep_alive` defaults to true; build the connection builder once and
    // clone it cheaply per accepted connection.
    let http1 = http1::Builder::new();

    // listen for requests until terminated
    loop {
        let (stream, _) = match listener.accept().await {
            Ok(conn) => conn,
            Err(error) => {
                // A transient accept error (e.g. file-descriptor exhaustion)
                // must not tear down the whole server.
                tracing::error!(%error, "accept error");
                continue;
            }
        };
        if let Err(error) = stream.set_nodelay(true) {
            tracing::warn!(%error, "failed to set TCP_NODELAY");
        }
        let stream = TokioIo::new(stream);
        let handler = handler.clone();
        let http1 = http1.clone();

        tokio::spawn(async move {
            if let Err(e) = http1
                .serve_connection(
                    stream,
                    service_fn(move |request| {
                        let handler = handler.clone();
                        async move {
                            let response = handler.handle(request).await.unwrap_or_else(|e| {
                                tracing::error!("Error proxying request: {e}");
                                internal_error()
                            });

                            // track server error responses
                            if response.status() >= StatusCode::INTERNAL_SERVER_ERROR {
                                tracing::error!(
                                    monotonic_counter.processing_errors = 1,
                                    service = %handler.component,
                                    error = format!("{response:?}"),
                                );
                            }
                            Ok::<_, Infallible>(response)
                        }
                    }),
                )
                .await
            {
                tracing::error!("connection error: {e:?}");
            }
        });
    }
}

#[derive(Clone)]
struct Handler<B>
where
    B: Clone + Send + Sync + 'static,
    StoreCtx<B>: WasiHttpView,
{
    state: Runtime<B>,
    component: Arc<str>,
    routing: Arc<TriggerRouter<ServiceIndices, HttpRoutes>>,
}

impl<B> Handler<B>
where
    B: Clone + Send + Sync + 'static,
    StoreCtx<B>: WasiHttpView,
{
    // Resolve an unrouted path through the runtime's shared late-routing
    // helper (`Runtime::route_http`: `http_paths` hook → `ensure_guest`,
    // resolve-on-miss included), then probe request-local handler indices
    // against the exact `InstancePre` this request instantiates. An `Err`
    // carries the ready 404/500 response: an unmatched path (no hook, the
    // hook declined, or the identity is one nothing supplies) is an ordinary
    // 404, while a claimed path that faults is a deployment fault — an error
    // + 500, never hidden as a miss.
    async fn route(
        &self, path: &str,
    ) -> std::result::Result<(Arc<Guest<StoreCtx<B>>>, ServiceIndices), hyper::Response<OutgoingBody>>
    {
        let guest = match self.state.route_http(path).await {
            Ok(guest) => guest,
            Err(RouteRefusal::NotFound) => return Err(not_found()),
            Err(refusal @ RouteRefusal::Failed(_)) => {
                let error = anyhow::Error::new(refusal);
                tracing::error!(path, "claimed http route faulted; returning 500: {error:#}");
                return Err(internal_error());
            }
        };
        // Belt-and-braces: `ensure_guest` already export-validates both the
        // resolved and the concurrently registered component, so this arm is
        // not expected to fire; keep the outcome aligned (error + 500) if it
        // ever does.
        let indices = match ServiceIndices::new(guest.instance_pre()) {
            Ok(indices) => indices,
            Err(error) => {
                tracing::error!(
                    path,
                    "claimed http route guest lacks the http handler; returning 500: {error:#}"
                );
                return Err(internal_error());
            }
        };
        Ok((guest, indices))
    }

    // Forward request to the wasm Guest.
    async fn handle(
        &self, request: hyper::Request<Incoming>,
    ) -> Result<hyper::Response<OutgoingBody>> {
        tracing::debug!("handling request: {request:?}");

        // Normalise the request (scheme/authority); a request we cannot
        // normalise (e.g. a missing `Host` header) is a client error, not a 500.
        let request = match fix_request(request) {
            Ok(request) => request,
            Err(error) => {
                tracing::debug!(%error, "rejecting malformed request");
                return Ok(bad_request());
            }
        };

        // Resolve the guest by request path: a static route hit dispatches
        // through the boot-built router; a miss consults the deployment's
        // `http_paths` hook, whose identity goes through `ensure_guest` (and
        // hence resolve-on-miss) with request-local handler indices probed
        // against the exact `InstancePre` this request instantiates.
        let guest;
        let late_indices;
        let indices: &ServiceIndices = if let Some((guest_id, indices)) =
            self.routing.resolve(request.uri().path())
        {
            // Static resolution only yields identities drawn from the
            // registry, so a miss is a lifecycle race (e.g. concurrent
            // deregistration) — a 500, never a server panic.
            if let Some(routed) = self.state.registry().get(guest_id) {
                guest = routed;
            } else {
                tracing::error!(guest = %guest_id, "routed guest is not registered; returning 500");
                return Ok(internal_error());
            }
            indices
        } else {
            match self.route(request.uri().path()).await {
                Ok((routed_guest, indices)) => {
                    guest = routed_guest;
                    late_indices = indices;
                    &late_indices
                }
                Err(response) => return Ok(response),
            }
        };

        // instantiate the selected guest fresh (instance-per-call)
        let store_data = self.state.store();
        let mut store = self.state.build_store(store_data);
        let instance = self.state.instantiate(guest.instance_pre(), &mut store).await?;
        let service = indices.load(&mut store, &instance)?;

        let (sender, receiver) = oneshot::channel::<Result<hyper::Response<OutgoingBody>>>();

        let guest_task = tokio::spawn(async move {
            let result = store
                .run_concurrent(async |store| {
                    // Build the guest's response, routing every failure (a trap,
                    // a guest-returned error, or a response-conversion error)
                    // through `sender`. A single error path means the caller
                    // never mistakes a real error for a panicked task.
                    let built = async move {
                        let (parts, body) = request.into_parts();
                        let body = body.map_err(ErrorCode::from_hyper_request_error);
                        let http_req = http::Request::from_parts(parts, body);
                        let (request, io) = wasi::Request::from_http(http_req);

                        let wasi_resp = service
                            .handle(store, request)
                            .await
                            .map_err(anyhow::Error::from)
                            .context("guest trap")?
                            .map_err(|e| anyhow!("guest error: {e}"))?;
                        store
                            .with(|mut store| wasi_resp.into_http(&mut store, io))
                            .map_err(|e| anyhow!("converting guest response: {e}"))
                    }
                    .await;

                    match built {
                        Ok(resp) => {
                            // wrap the body so we can detect when hyper finishes
                            // consuming it, then keep run_concurrent alive until
                            // it does so the WASI pipe resources stay valid
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
                .instrument(debug_span!("http-request"))
                .await;

            match result {
                Ok(Ok(())) => {}
                Ok(Err(e)) => tracing::error!("http guest task error: {e:#}"),
                Err(e) => tracing::error!("run_concurrent error: {e:?}"),
            }
        });

        // bound time-to-response (not the streaming body); cancel a hung guest
        let response = match timeout(self.state.options().guest_timeout, receiver).await {
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

// Prepare the request for the guest.
fn fix_request(mut request: hyper::Request<Incoming>) -> Result<hyper::Request<Incoming>> {
    // rebuild Uri with scheme and authority explicitly set so they are passed to the Guest
    let uri = request.uri_mut();
    let p_and_q = uri.path_and_query().map_or_else(|| PathAndQuery::from_static("/"), Clone::clone);
    let mut uri_builder = Uri::builder().path_and_query(p_and_q);

    if let Some(forwarded) = request.headers().get(FORWARDED) {
        // Behind a proxy: RFC 7239 `Forwarded` is a comma-separated list of
        // elements (one per hop; the first is the client-facing one), each a
        // semicolon-separated set of parameters with case-insensitive names
        // and optionally quoted values.
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
        // running locally
        let Some(host) = request.headers().get(HOST) else {
            return Err(anyhow!("missing host header"));
        };
        uri_builder = uri_builder.authority(host.to_str()?);
        uri_builder = uri_builder.scheme("http");
    }

    // update the uri with the new scheme and authority
    let (mut parts, body) = request.into_parts();
    parts.uri = uri_builder.build()?;
    let request = hyper::Request::from_parts(parts, body);

    Ok(request)
}

/// Wraps a response body and holds a `oneshot::Sender` that is dropped when
/// the body is fully consumed (or the wrapper itself is dropped). The
/// corresponding receiver keeps `run_concurrent` alive so WASI pipe resources
/// remain valid while hyper streams the response.
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

/// A `400 Bad Request` for a request the server could not normalise (e.g. a
/// missing `Host` header).
fn bad_request() -> hyper::Response<OutgoingBody> {
    let body = Full::new(Bytes::from_static(b"Bad Request")).map_err(Into::into).boxed_unsync();

    hyper::Response::builder()
        .status(StatusCode::BAD_REQUEST)
        .header("Content-Type", "text/plain; charset=UTF-8")
        .body(body)
        .expect("should build bad request response")
}

/// A `404 Not Found` for a request that matched no route (or a trigger with no
/// http-capable guest).
fn not_found() -> hyper::Response<OutgoingBody> {
    let body = Full::new(Bytes::from_static(b"Not Found")).map_err(Into::into).boxed_unsync();

    hyper::Response::builder()
        .status(StatusCode::NOT_FOUND)
        .header("Content-Type", "text/plain; charset=UTF-8")
        .body(body)
        .expect("should build not found response")
}
