use std::any::Any;
use std::error::Error;

use anyhow::{Context, Result};
use bytes::{Bytes, BytesMut};
use http::HeaderValue;
use http::header::{CONTENT_LENGTH, ETAG};
use http_body::Body;
use wasip3::http::handler;
use wasip3::http_compat::{IncomingMessage, http_from_wasi_response, http_into_wasi_request};
use wasip3::wit_bindgen::StreamResult;
use wasip3::wit_future;

pub use crate::guest::cache::{Cache, CacheOptions};

const CHUNK_SIZE: usize = 2048;

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
    let wasi_resp = handler::handle(wasi_req).await.context("Issue calling proxy")?;
    let http_resp = http_from_wasi_response(wasi_resp).context("Issue converting response")?;

    // convert wasi response to http response
    let (parts, mut body) = http_resp.into_parts();

    // read body
    let mut body_buf = BytesMut::new();
    if let Some(len) = parts.headers.get(CONTENT_LENGTH)
        && let Ok(cl) = len.to_str()
    {
        let len = cl.parse::<usize>().unwrap_or(0);
        body_buf.reserve(len);
    }

    if let Some(response) = body.take_unstarted() {
        let (_, body_rx) = wit_future::new(|| Ok(()));
        let (mut stream, _trailers) = response.consume_body(body_rx);

        loop {
            let read_buf = Vec::with_capacity(CHUNK_SIZE);
            let (result, read) = stream.read(read_buf).await;
            body_buf.extend_from_slice(&read);

            let StreamResult::Complete(_) = result else {
                tracing::debug!("body read cancelled or dropped");
                break;
            };
        }
    }

    let mut response = http::Response::from_parts(parts, body_buf.into());

    // cache response when indicated by `Cache-Control` header
    if let Some(cache) = maybe_cache {
        response.headers_mut().insert(ETAG, HeaderValue::from_str(&cache.etag())?);
        cache.put(&response).await?;
        tracing::debug!("response cached");
    }

    tracing::debug!("proxy response: {response:?}");

    Ok(response)
}
