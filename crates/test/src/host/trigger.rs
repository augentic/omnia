//! Drives one request or one delivery through a booted runtime's guests, in
//! process.

use anyhow::{Result, bail};
use bytes::Bytes;
use http::header::HOST;
use http::uri::Authority;
use http::{HeaderValue, Request, Response};
use http_body_util::{BodyExt as _, Full};
use omnia::{Provides, Runtime};
use omnia_wasi_http::{HttpHandler, WasiHttp};
use omnia_wasi_messaging::{HandleError, Message, MessagingHandler, WasiMessaging};

/// Serve `request` through the guest `runtime` routes it to, and collect the
/// reply's body.
///
/// A request without a `Host` header is given one, since the host refuses a
/// request without. It names the URI's authority, or `localhost` when the URI
/// has none. An unrouted path is a `404` reply, not an error.
///
/// # Errors
///
/// Returns an error if no guest exports `wasi:http`, the deployment's routes
/// are inconsistent with the guests' exports, the guest cannot be
/// instantiated or traps, or the reply's body fails part way.
pub async fn send<B: Provides<WasiHttp>>(
    runtime: &Runtime<B>, mut request: Request<Bytes>,
) -> Result<Response<Bytes>> {
    let Some(handler) = HttpHandler::new(runtime)? else {
        bail!("no guest exports `wasi:http/handler`");
    };
    if !request.headers().contains_key(HOST) {
        let host = request.uri().authority().map_or("localhost", Authority::as_str);
        let host = HeaderValue::from_str(host)?;
        request.headers_mut().insert(HOST, host);
    }

    let response = handler.handle(request.map(Full::new)).await?;
    let (parts, body) = response.into_parts();
    let body = body.collect().await?.to_bytes();
    Ok(Response::from_parts(parts, body))
}

/// Deliver `payload` on `topic` to the guest `runtime` routes the topic to.
///
/// The delivery carries no metadata, so the guest sees no key and no content
/// type. A topic no guest is routed to is dropped with `Ok`.
///
/// # Errors
///
/// Returns the [`HandleError`] the guest settles the delivery with, or
/// [`HandleError::Unavailable`] when no guest exports the messaging handler
/// or the deployment's routes are inconsistent with the guests' exports.
pub async fn deliver<B: Provides<WasiMessaging>>(
    runtime: &Runtime<B>, topic: &str, payload: impl Into<Vec<u8>>,
) -> Result<(), HandleError> {
    let handler = match MessagingHandler::new(runtime) {
        Ok(Some(handler)) => handler,
        Ok(None) => {
            let missing = "no guest exports the messaging handler".to_owned();
            return Err(HandleError::Unavailable(missing));
        }
        Err(err) => return Err(HandleError::Unavailable(format!("{err:#}"))),
    };
    let mut message = Message::new(payload.into());
    topic.clone_into(&mut message.topic);
    handler.handle(message).await
}
