use std::any::Any;
use std::error::Error;

use anyhow::{Context, Result};
use bytes::Bytes;
use http::HeaderValue;
use http::header::ETAG;
use http_body::Body;
use wasip3::http::client;
use wasip3::http_compat::{IncomingMessage, http_from_wasi_response, http_into_wasi_request};
use wasip3::wit_future;

pub use crate::guest::cache::{Cache, CacheOptions};

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
    let maybe_cache = Cache::maybe_from(&request)?;

    // check cache when indicated by `Cache-Control` header
    if let Some(cache) = maybe_cache.as_ref()
        && let Some(hit) = cache.get().await?
    {
        tracing::debug!("cache hit");
        return Ok(hit);
    }

    // forward to `wasmtime-wasi-http` outbound proxy
    tracing::debug!("forwarding request to proxy: {:?}", request.headers());
    let wasi_req = http_into_wasi_request(request).context("Issue converting request")?;
    let wasi_resp = client::send(wasi_req).await.context("Issue calling proxy")?;
    let http_resp = http_from_wasi_response(wasi_resp).context("Issue converting response")?;

    // convert wasi response to http response
    let (parts, mut body) = http_resp.into_parts();

    // read body
    let bytes: Vec<u8> = if let Some(response) = body.take_unstarted() {
        let (_, body_rx) = wit_future::new(|| Ok(()));
        let (stream, _trailers) = response.consume_body(body_rx);

        stream.collect().await
    } else {
        vec![]
    };

    let mut response = http::Response::from_parts(parts, bytes.into());

    // cache response when indicated by `Cache-Control` header
    if let Some(cache) = maybe_cache {
        response.headers_mut().insert(ETAG, HeaderValue::from_str(&cache.etag())?);
        cache.put(&response).await?;
        tracing::debug!("response cached");
    }

    tracing::debug!("proxy response: {response:?}");

    Ok(response)
}
