use std::any::Any;
use std::error::Error;

use anyhow::{Context, Result};
use bytes::Bytes;
use http_body::Body;
use wasip3::http::client;
use wasip3::http_compat::{IncomingMessage, http_from_wasi_response, http_into_wasi_request};
use wasip3::wit_future;

/// Send an HTTP request using the WASI HTTP proxy handler.
///
/// # Errors
///
/// Returns an error if the request could not be sent.
pub async fn handle<T>(request: http::Request<T>) -> Result<http::Response<Bytes>>
where
    T: Body + Any,
    T::Data: Into<Vec<u8>>,
    T::Error: Into<Box<dyn Error + Send + Sync + 'static>>,
{
    // forward to `wasmtime-wasi-http` outbound proxy
    tracing::debug!("forwarding request to proxy: {:?}", request.headers());
    let wasi_req = http_into_wasi_request(request).context("Issue converting request")?;
    let wasi_resp = client::send(wasi_req).await.context("Issue calling proxy")?;
    let http_resp = http_from_wasi_response(wasi_resp).context("Issue converting response")?;

    let (parts, mut body) = http_resp.into_parts();

    let bytes: Vec<u8> = if let Some(response) = body.take_unstarted() {
        let (_, body_rx) = wit_future::new(|| Ok(()));
        let (stream, _trailers) = response.consume_body(body_rx);

        stream.collect().await
    } else {
        vec![]
    };

    let response = http::Response::from_parts(parts, bytes.into());
    tracing::debug!("proxy response: {response:?}");

    Ok(response)
}
