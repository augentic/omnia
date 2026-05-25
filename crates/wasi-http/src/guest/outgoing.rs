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

/// Per-request resilience policy.
///
/// Attach to a request via extensions before calling [`handle`]. The guest runtime
/// serializes this into internal headers that the host reads and strips — the upstream
/// never sees them.
///
/// ```rust,ignore
/// request.extensions_mut().insert(OutboundPolicy {
///     timeout_ms: Some(5000),
///     upstream: None,
/// });
/// ```
#[derive(Clone, Debug, Default)]
pub struct OutboundPolicy {
    /// Response timeout in milliseconds. Falls back to host default if `None`.
    pub timeout_ms: Option<u64>,
    /// Override breaker bucket name. Falls back to the default breaker if `None`.
    pub upstream: Option<String>,
}

/// Send an HTTP request using the WASI HTTP proxy handler.
///
/// # Errors
///
/// Returns an error if the request could not be sent.
pub async fn handle<T>(mut request: http::Request<T>) -> Result<http::Response<Bytes>>
where
    T: Body + Any,
    T::Data: Into<Vec<u8>>,
    T::Error: Into<Box<dyn Error + Send + Sync + 'static>>,
{
    let maybe_cache = Cache::maybe_from(&request)?;

    // Serialize OutboundPolicy into headers before crossing the WASI boundary
    if let Some(policy) = request.extensions_mut().remove::<OutboundPolicy>() {
        if let Some(ms) = policy.timeout_ms {
            request.headers_mut().insert("x-omnia-timeout-ms", HeaderValue::from(ms));
        }
        if let Some(ref name) = policy.upstream {
            if let Ok(val) = HeaderValue::from_str(name) {
                request.headers_mut().insert("x-omnia-upstream", val);
            }
        }
    }

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
