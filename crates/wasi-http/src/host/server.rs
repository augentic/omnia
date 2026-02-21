//! #HTTP Server

use std::clone::Clone;
use std::convert::Infallible;
use std::env;
use std::sync::Arc;

use anyhow::{Context, Result, anyhow};
use bytes::Bytes;
use http::StatusCode;
use http::uri::{PathAndQuery, Uri};
use http_body_util::combinators::UnsyncBoxBody;
use http_body_util::{BodyExt, Full};
use hyper::body::Incoming;
use hyper::header::{FORWARDED, HOST};
use hyper::server::conn::http1;
use hyper::service::service_fn;
use omnia::State;
use tokio::net::TcpListener;
use tokio::sync::oneshot;
use tracing::{Instrument, debug_span};
use wasmtime::Store;
use wasmtime_wasi_http::io::TokioIo;
use wasmtime_wasi_http::p3::WasiHttpView;
use wasmtime_wasi_http::p3::bindings::ServiceIndices;
use wasmtime_wasi_http::p3::bindings::http::types::{self as wasi, ErrorCode};

type OutgoingBody = UnsyncBoxBody<Bytes, anyhow::Error>;

const HTTP_ADDR: &str = "0.0.0.0:8080";

pub async fn serve<S>(state: &S) -> Result<()>
where
    S: State,
    S::StoreCtx: WasiHttpView,
{
    let component = env::var("COMPONENT").unwrap_or_else(|_| "unknown".into());
    let addr = env::var("HTTP_ADDR").unwrap_or_else(|_| HTTP_ADDR.into());

    let listener = TcpListener::bind(&addr).await?;
    tracing::info!("{component} http server listening on: {addr}");

    let handler = Handler {
        state: Arc::new(state.clone()),
        component,
    };

    // listen for requests until terminated
    loop {
        let (stream, _) = listener.accept().await?;
        stream.set_nodelay(true)?;
        let stream = TokioIo::new(stream);
        let handler = handler.clone();

        tokio::spawn(async move {
            let mut http1 = http1::Builder::new();
            http1.keep_alive(true);

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
struct Handler<S>
where
    S: State,
    S::StoreCtx: WasiHttpView,
{
    state: Arc<S>,
    component: String,
}

impl<S> Handler<S>
where
    S: State,
    S::StoreCtx: WasiHttpView,
{
    // Forward request to the wasm Guest.
    async fn handle(
        &self, request: hyper::Request<Incoming>,
    ) -> Result<hyper::Response<OutgoingBody>> {
        tracing::debug!("handling request: {request:?}");

        // prepare wasmtime http request and response
        let request = fix_request(request).context("preparing request")?;

        // instantiate the guest and get the proxy
        let instance_pre = self.state.instance_pre();
        let store_data = self.state.store();
        let mut store = Store::new(instance_pre.engine(), store_data);
        let indices = ServiceIndices::new(instance_pre)?;
        let instance = instance_pre.instantiate_async(&mut store).await?;
        let service = indices.load(&mut store, &instance)?;

        let (sender, receiver) = oneshot::channel();

        tokio::spawn(async move {
            let guest_result = store
                .run_concurrent(async |store| {
                    // convert hyper::Request to wasi::Request
                    let (parts, body) = request.into_parts();
                    let body = body.map_err(ErrorCode::from_hyper_request_error);
                    let http_req = http::Request::from_parts(parts, body);
                    let (request, io_result) = wasi::Request::from_http(http_req);

                    // forward request to guest
                    let (wasi_resp, task) = service.handle(store, request).await??;
                    let http_resp =
                        store.with(|mut store| wasi_resp.into_http(&mut store, io_result))?;
                    _ = sender.send(http_resp);
                    task.block(store).await;

                    anyhow::Ok(())
                })
                .instrument(debug_span!("http-request"))
                .await?;

            if let Err(e) = guest_result {
                tracing::error!("Guest error: {e:?}");
                return Err(e);
            }

            // write_profile(&mut store);
            // drop(epoch_thread);

            Ok(())
        });

        let response = receiver.await?.map(|body| body.map_err(Into::into).boxed_unsync());
        tracing::debug!("received response: {response:?}");

        Ok(response)
    }
}

// Prepare the request for the guest.
fn fix_request(mut request: hyper::Request<Incoming>) -> Result<hyper::Request<Incoming>> {
    // let req_id = self.next_id.fetch_add(1, Ordering::Relaxed);

    // rebuild Uri with scheme and authority explicitly set so they are passed to the Guest
    let uri = request.uri_mut();
    let p_and_q = uri.path_and_query().map_or_else(|| PathAndQuery::from_static("/"), Clone::clone);
    let mut uri_builder = Uri::builder().path_and_query(p_and_q);

    if let Some(forwarded) = request.headers().get(FORWARDED) {
        // running behind a proxy (that we have configured)
        for tuple in forwarded.to_str()?.split(';') {
            let tuple = tuple.trim();
            if let Some(host) = tuple.strip_prefix("host=") {
                uri_builder = uri_builder.authority(host);
            } else if let Some(proto) = tuple.strip_prefix("proto=") {
                uri_builder = uri_builder.scheme(proto);
            }
        }
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
        .status(500)
        .header("Content-Type", "text/html; charset=UTF-8")
        .body(body)
        .expect("should build internal error response")
}
