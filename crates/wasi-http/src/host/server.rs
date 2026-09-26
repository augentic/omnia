//! #HTTP Server

use std::clone::Clone;
use std::convert::Infallible;
use std::env;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context as TaskContext, Poll};

use anyhow::{Context, Result, anyhow};
use bytes::Bytes;
use http::StatusCode;
use http::uri::{PathAndQuery, Uri};
use http_body_util::combinators::UnsyncBoxBody;
use http_body_util::{BodyExt, Full};
use hyper::body::{Body, Frame, SizeHint};
use hyper::header::{FORWARDED, HOST};
use hyper::server::conn::http1;
use hyper::service::service_fn;
use omnia_core::{HttpRoutes, Runtime, StoreCtx, TriggerRouter};
use tokio::net::TcpListener;
use tokio::sync::oneshot;
use tokio::time::timeout;
use tracing::{Instrument, info_span, instrument};
use wasmtime::AsContextMut as _;
use wasmtime_wasi_http::WasiHttpView;
use wasmtime_wasi_http::io::TokioIo;
use wasmtime_wasi_http::p3::bindings::ServiceIndices;
use wasmtime_wasi_http::p3::bindings::http::types as wasi;

/// The streaming body of a response produced by [`HttpHandler::handle`].
pub type OutgoingBody = UnsyncBoxBody<Bytes, anyhow::Error>;

const HTTP_ADDR: &str = "0.0.0.0:8080";

#[instrument("http-server", skip(state))]
pub async fn run<B>(state: &Runtime<B>) -> Result<()>
where
    B: Clone + Send + Sync + 'static,
    StoreCtx<B>: WasiHttpView,
{
    let Some(handler) = HttpHandler::new(state)? else {
        tracing::info!("no guest exports the http handler; http trigger inert");
        return Ok(());
    };

    let addr = env::var("HTTP_ADDR").unwrap_or_else(|_| HTTP_ADDR.into());
    let listener = TcpListener::bind(&addr).await.with_context(|| format!("binding {addr}"))?;
    let addr = listener.local_addr().context("reading http listener address")?;
    tracing::info!("{} http server listening on: {addr}", handler.component);

    // built once, cloned per connection
    let http1 = http1::Builder::new();

    // listen for requests until terminated
    loop {
        let (stream, _) = match listener.accept().await {
            Ok(conn) => conn,
            Err(error) => {
                // a transient accept error must not tear down the server
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

/// In-process `wasi:http/incoming-handler` dispatcher: routes a request to a
/// guest, instantiates it, and invokes the export.
///
/// The server loop drives one over accepted connections; tests drive it
/// directly with a constructed request, bypassing the socket.
#[derive(Clone)]
pub struct HttpHandler<B>
where
    B: Clone + Send + Sync + 'static,
    StoreCtx<B>: WasiHttpView,
{
    state: Runtime<B>,
    component: Arc<str>,
    routing: Arc<TriggerRouter<HttpRoutes>>,
}

impl<B> HttpHandler<B>
where
    B: Clone + Send + Sync + 'static,
    StoreCtx<B>: WasiHttpView,
{
    /// Build the handler over `runtime`'s registry; `None` when no guest
    /// exports the http handler (the trigger is inert).
    ///
    /// # Errors
    ///
    /// Returns an error if the deployment's http routes are inconsistent with
    /// the guests' exports.
    pub fn new(runtime: &Runtime<B>) -> Result<Option<Self>> {
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
        }))
    }

    /// Forward a request to the routed guest and return its response.
    ///
    /// The request is normalised first (scheme and authority from `Host` or
    /// `Forwarded`). Outcomes the handler can answer itself come back as
    /// responses: a request without either header `400`, an unrouted path
    /// `404`, and a guest that times out, cannot be loaded at its first use,
    /// or is no longer registered `500`.
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

        // normalise the request; a bad `Host` header is a 400, not a 500
        let request = match fix_request(request) {
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
        let indices = ServiceIndices::new(guest.instance_pre())
            .map_err(anyhow::Error::from)
            .with_context(|| {
                format!("routed guest `{guest_id}` does not export `wasi:http/incoming-handler`")
            })?;

        // instantiate the selected guest fresh (instance-per-call)
        let store_data = self.state.store();
        let mut store = self.state.build_store(store_data);
        let instance = self.state.instantiate(guest.instance_pre(), &mut store).await?;
        let service = indices.load(&mut store, &instance)?;

        let (sender, receiver) = oneshot::channel::<Result<hyper::Response<OutgoingBody>>>();

        let guest_task = tokio::spawn(async move {
            let result = store
                .run_concurrent(async |store| {
                    // one error path through `sender`, so the caller never
                    // mistakes a real error for a panicked task
                    let built = async move {
                        let (request, io) = store.with(|mut access| {
                            let mut cx = access.as_context_mut();
                            wasi::Request::from_http(cx.data_mut().http().hooks, request)
                        });

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

// rebuild the uri with scheme and authority set, so they reach the guest
fn fix_request<T>(mut request: http::Request<T>) -> Result<http::Request<T>> {
    let uri = request.uri_mut();
    let p_and_q = uri.path_and_query().map_or_else(|| PathAndQuery::from_static("/"), Clone::clone);
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
        // running locally
        let Some(host) = request.headers().get(HOST) else {
            return Err(anyhow!("missing host header"));
        };
        uri_builder = uri_builder.authority(host.to_str()?);
        uri_builder = uri_builder.scheme("http");
    }

    let (mut parts, body) = request.into_parts();
    parts.uri = uri_builder.build()?;
    let request = http::Request::from_parts(parts, body);

    Ok(request)
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
