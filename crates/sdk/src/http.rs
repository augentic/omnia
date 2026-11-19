//! `wasi:http` for a guest: outbound requests, and inbound ones served by a router.
//!
//! - [`fetch`] sends a request whose body is [`Bytes`], and [`fetch_json`]
//!   reads its JSON reply.
//! - [`send`] takes any body, and fails with the host's own error.
//! - `serve` answers an inbound request through an axum router, on `wasm32`
//!   with the `http` feature.

use std::any::Any;
use std::error::Error as StdError;

use anyhow::Context as _;
use bytes::Bytes;
use http::{Request, Response};
use http_body::Body;
use http_body_util::Full;
use serde::de::DeserializeOwned;
use wasip3::http::client;
use wasip3::http_compat::{IncomingMessage as _, http_from_wasi_response, http_into_wasi_request};

use crate::{Error, Result};

/// Send `request` through the host's outbound client and return the response.
///
/// Any status is a response, not an error.
///
/// # Errors
///
/// [`BadGateway`](Error::BadGateway) with the code `unavailable` when the
/// upstream does not answer.
pub async fn fetch(request: Request<Bytes>) -> Result<Response<Bytes>> {
    let target = format!("{} {}", request.method(), request.uri());
    send(request.map(Full::new))
        .await
        .map_err(|err| Error::bad_gateway("unavailable", format!("sending {target}: {err:#}")))
}

/// Send `request`, require a 2xx status, and deserialize the body as JSON.
///
/// # Errors
///
/// [`BadGateway`](Error::BadGateway) when the upstream does not answer,
/// returns a non-2xx status, or returns a body that is not the JSON of an `O`.
pub async fn fetch_json<O: DeserializeOwned>(request: Request<Bytes>) -> Result<O> {
    json(fetch(request).await?)
}

fn json<O: DeserializeOwned>(response: Response<Bytes>) -> Result<O> {
    if !response.status().is_success() {
        return Err(crate::bad_gateway!("upstream returned {}", response.status()));
    }
    serde_json::from_slice(&response.into_body())
        .map_err(|error| crate::bad_gateway!("parsing upstream response: {error}"))
}

/// Send one request through the host's outbound `wasi:http` client.
///
/// The response's body is read in full. A request's `request-options` ride
/// its `RequestOptionsExtension`.
///
/// # Errors
///
/// Returns an error when the host refuses or fails the request. It downcasts
/// to the `wasi:http` `ErrorCode` the host reported.
pub async fn send<T>(request: Request<T>) -> anyhow::Result<Response<Bytes>>
where
    T: Body + Any,
    T::Data: Into<Vec<u8>>,
    T::Error: Into<Box<dyn StdError + Send + Sync + 'static>>,
{
    tracing::trace!("forwarding request to proxy: {:?}", request.headers());
    let request = http_into_wasi_request(request).context("Issue converting request")?;
    let response = client::send(request).await.context("Issue calling proxy")?;
    let response = http_from_wasi_response(response).context("Issue converting response")?;

    // read the body in full
    let (parts, mut body) = response.into_parts();
    let bytes: Vec<u8> = if let Some(response) = body.take_unstarted() {
        let (_, body_rx) = wasip3::wit_future::new(|| Ok(()));
        let (stream, _trailers) = response.consume_body(body_rx);
        stream.collect().await
    } else {
        vec![]
    };

    let response = Response::from_parts(parts, bytes.into());
    tracing::trace!("proxy response: {response:?}");
    Ok(response)
}

/// Serve one `wasi:http` request through `router`.
///
/// The request runs inside the guest telemetry scope and the `handler` span
/// [`trace`](crate::api::http::trace) opens, so a router served here does not
/// layer `trace` itself. [`http::export!`](crate::api::http::export!) expands
/// to this call, and a hand-written `wasi:http` export calls it directly.
///
/// # Errors
///
/// Returns the WASI error code when the request or the router's response
/// does not convert.
#[cfg(all(feature = "http", target_arch = "wasm32"))]
#[crate::otel::instrument(name = "http_guest_handle")]
pub async fn serve(
    router: impl Into<axum::Router>, request: wasip3::http::types::Request,
) -> Result<wasip3::http::types::Response, wasip3::http::types::ErrorCode> {
    use wasip3::http::types::ErrorCode;
    use wasip3::http_compat::{http_from_wasi_request, http_into_wasi_response};

    let router = router.into().layer(axum::middleware::from_fn(crate::api::http::trace));
    let request = http_from_wasi_request(request)?;
    tracing::trace!("serving request: {:?}", request.headers());

    let response = tower::ServiceExt::oneshot(router, request)
        .await
        .map_err(|e| ErrorCode::InternalError(Some(format!("issue processing request: {e}"))))?;
    tracing::trace!("guest response: {response:?}");
    http_into_wasi_response(response)
}
