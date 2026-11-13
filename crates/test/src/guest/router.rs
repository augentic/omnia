//! Drives a guest router natively: one request or one delivery through it.

use std::convert::Infallible;
use std::fmt;

use bytes::Bytes;
use http::{Request, Response};
use http_body::Body;
use http_body_util::BodyExt as _;
use omnia_sdk::api::Router;
use omnia_sdk::api::messaging::{Delivery, DeliveryError};
use tower::{Service, ServiceExt as _};

/// Send one request through `service` and collect the reply's body.
///
/// `service` is the router under test, an `axum::Router` or an
/// [`omnia_sdk::api::Router`]. It is cloned for the one call, so a test
/// keeps driving the same router.
///
/// # Panics
///
/// Panics when the reply's body cannot be read.
pub async fn send<S, B, RB>(service: &S, request: Request<B>) -> Response<Bytes>
where
    S: Service<Request<B>, Response = Response<RB>, Error = Infallible> + Clone + Send + Sync,
    S::Future: Send,
    B: Send,
    RB: Body<Data = Bytes> + Send,
    RB::Error: fmt::Debug,
{
    let Ok(response) = service.clone().oneshot(request).await;
    let (parts, body) = response.into_parts();
    let body = body.collect().await.expect("the reply's body is readable").to_bytes();
    Response::from_parts(parts, body)
}

/// Deliver `payload` on `topic` through `router`.
///
/// The delivery carries no content type and no metadata, so the handler sees
/// a minted request id, as it does for a host delivery without one.
///
/// # Errors
///
/// Returns the router's [`DeliveryError`]: the topic is unhandled, the payload
/// does not decode, or the handler failed.
pub async fn deliver<P>(
    router: &Router<P>, topic: &str, payload: impl Into<Vec<u8>>,
) -> Result<(), DeliveryError>
where
    P: Send + Sync + 'static,
{
    router.handle(Delivery::new(topic, &payload.into())).await
}
