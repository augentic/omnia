//! `wasi:http` for a guest: outbound requests, and inbound ones served by a router.
//!
//! - [`get`], [`post`], [`put`], [`patch`], and [`delete`] start a
//!   [`RequestBuilder`], which sends what it builds through [`fetch`] or
//!   [`fetch_json`].
//! - [`fetch`] sends a request whose body is [`Bytes`], and [`fetch_json`]
//!   reads its JSON reply.
//! - [`send`] takes any body, and fails with the host's own error.
//! - With the `http` feature, [`cache::HttpCache`] fetches through a
//!   keyvalue bucket where a request's `Cache-Control` asks.
//!
//! With the `http` feature, a route is an ordinary axum handler on a
//! [`Handlers`](crate::Handlers). It reads the call's
//! [`Metadata`](crate::Metadata) through the [`Context`](crate::Context)
//! extractor and its input with axum's `Path`, `Query`, and `Json`
//! extractors. It answers a `Result` whose error is an [`Error`], the JSON
//! [`ErrorBody`](crate::ErrorBody) at its status. `?` on an `anyhow::Error`
//! reaches the same envelope, and a domain error with a wire shape of its own
//! implements `IntoResponse` itself. [`export!`] binds the router fn as the
//! guest's `wasi:http` handler, and `serve` answers each request inside the
//! `handler` span [`trace`] opens.

#[cfg(feature = "http")]
pub mod cache;
mod request;
#[cfg(feature = "http")]
mod server;

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

pub use self::request::{RequestBuilder, delete, get, patch, post, put};
#[cfg(all(feature = "http", target_arch = "wasm32"))]
pub use self::server::serve;
#[cfg(feature = "http")]
pub use self::server::trace;
use crate::{Error, Result};

/// Send `request` through the host's outbound client and return the response.
///
/// Any status is a response, not an error.
///
/// # Errors
///
/// [`BadGateway`](crate::ErrorKind::BadGateway) with the code `unavailable` when the
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
/// [`BadGateway`](crate::ErrorKind::BadGateway) when the upstream does not answer,
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
    tracing::trace!("sending request: {:?}", request.headers());
    let request = http_into_wasi_request(request).context("converting request")?;
    let response = client::send(request).await.context("calling the outbound client")?;
    let response = http_from_wasi_response(response).context("converting response")?;

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
    tracing::trace!("response: {response:?}");
    Ok(response)
}

/// Bind a router fn as the guest's `wasi:http` export.
///
/// Calls the router fn on each request and serves the request through the
/// router it returns, a [`Router`](crate::Router) or an `axum::Router`,
/// inside the `handler` span [`trace`] opens. On a native target it binds
/// nothing, and a private router fn still counts as used.
///
/// ```rust,ignore
/// omnia_sdk::http::export!(router);
/// ```
#[cfg(feature = "http")]
#[doc(inline)]
pub use crate::__export_http as export;
