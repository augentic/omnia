//! An outbound response cache over an `omnia:keyvalue` bucket.
//!
//! [`HttpCache::fetch`] sends a request through [`fetch`](super::fetch), and
//! answers it from the bucket where its `Cache-Control` asks.
//! [`HttpCache::fetch_json`] reads the JSON of either answer. Caching is
//! opt-in per call site. A guest fetches through an [`HttpCache`] where it
//! wants cached responses, and through [`fetch`](super::fetch) elsewhere.
//!
//! The cache acts on three `Cache-Control` directives:
//!
//! - `max-age=<secs>` serves a stored response, or stores a successful one
//!   for that long. `max-age=0` does neither, since a request's `max-age` is
//!   the oldest response it accepts (RFC 9111 §5.2.1.1).
//! - `no-cache` goes to the origin past any stored response. Beside a
//!   `max-age` it stores the origin's response, a forced refresh.
//! - `no-store` goes to the origin and stores nothing.
//!
//! A request without `Cache-Control` passes through as sent. So does one
//! whose `Cache-Control` holds none of the three, since a cache ignores a
//! directive it does not recognise (RFC 9111 §5.2.3). Repeated
//! `Cache-Control` lines read as one list (RFC 9110 §5.3).
//!
//! The cache fails a request with the code `invalid`, before it leaves, when:
//!
//! - `no-store` meets `no-cache` or any `max-age`, in either order.
//! - `max-age` appears twice (RFC 9111 §4.2.1).
//! - `max-age` or `no-cache` comes without an `If-None-Match` of exactly one
//!   strong entity-tag (RFC 9110 §8.8.3). A weak tag, `*`, a bare token, and
//!   a list fail, and a second field line is a list. A comma inside the
//!   quotes belongs to the tag, so `"v1,v2"` is one entity-tag.
//! - The entity-tag holds `obs-text` octets, since [`Control::etag`] answers
//!   a `&str`.
//!
//! For a request the cache acts on:
//!
//! - `If-None-Match` is not forwarded, so the origin answers with the full
//!   resource.
//! - The response's `ETag` is the request's entity-tag, on a hit and a miss
//!   alike. Under `no-store` the origin's `ETag` stands.
//! - Only a complete 2xx response is stored, at [`store_key`] of the
//!   entity-tag, for `max-age` seconds. A `206 Partial Content` is returned
//!   unstored, since RFC 9111 §3 lets only a cache that tracks ranges store
//!   one.
//! - Header values are stored as bytes, so `obs-text` survives (RFC 9110
//!   §5.5).
//! - The store never fails a request. A failed or corrupt read is a miss, and
//!   a failed write returns the origin's response unstored. Both log at
//!   `warn`.

mod control;
mod envelope;

use std::time::Duration;

use bytes::Bytes;
use http::header::{ETAG, IF_NONE_MATCH};
use http::{HeaderValue, Request, Response, StatusCode};
use serde::de::DeserializeOwned;
use sha2::{Digest, Sha256};

pub use self::control::Control;
use self::envelope::{deserialize, serialize};
use crate::keyvalue::Bucket;
use crate::{Error, Result};

/// An outbound HTTP response cache over a keyvalue [`Bucket`].
///
/// # Examples
///
/// ```rust,no_run
/// use bytes::Bytes;
/// use http::Request;
/// use http::header::{CACHE_CONTROL, IF_NONE_MATCH};
/// use omnia_sdk::http::cache::HttpCache;
/// use omnia_sdk::keyvalue;
///
/// # async fn cached() -> anyhow::Result<()> {
/// let cache = HttpCache::new(keyvalue::default().await?);
/// let request = Request::get("https://origin.example/resource")
///     .header(CACHE_CONTROL, "max-age=300")
///     .header(IF_NONE_MATCH, "\"v1\"")
///     .body(Bytes::new())?;
/// let response = cache.fetch(request).await?;
/// # Ok(())
/// # }
/// ```
#[derive(Debug)]
pub struct HttpCache {
    bucket: Bucket,
}

impl HttpCache {
    /// A cache storing responses in `bucket`.
    #[must_use]
    pub const fn new(bucket: Bucket) -> Self {
        Self { bucket }
    }

    /// Send `request` to its origin, or answer it from the bucket.
    ///
    /// The header contract in the [module docs](crate::http::cache) decides
    /// which, and what is stored.
    ///
    /// # Errors
    ///
    /// - [`ServerError`](crate::ErrorKind::ServerError) with the code `invalid` for
    ///   malformed or conflicting caching headers, before any request leaves.
    /// - [`BadGateway`](crate::ErrorKind::BadGateway) with the code `unavailable` when
    ///   the origin does not answer.
    pub async fn fetch(&self, mut request: Request<Bytes>) -> Result<Response<Bytes>> {
        // pass through a request the cache does not act on
        let Some(control) = Control::maybe_from(request.headers())? else {
            return super::fetch(request).await;
        };

        // serve a stored response
        let etag = control.etag();
        if control.reads()
            && let Some(hit) = self.read_entry(etag).await
        {
            return Ok(hit);
        }

        // ask the origin for the full resource
        request.headers_mut().remove(IF_NONE_MATCH);
        let mut response = super::fetch(request).await?;

        // stamp the request's entity-tag
        if !etag.is_empty() {
            let value = HeaderValue::from_str(etag).map_err(|e| {
                Error::server_error("invalid", format!("etag `{etag}` is not a header value: {e}"))
            })?;
            response.headers_mut().insert(ETAG, value);
        }

        // store a complete success
        if control.writes() && storable(response.status()) {
            self.write_entry(etag, &response, control.max_age()).await;
        }

        Ok(response)
    }

    /// Answer `request` as [`fetch`](Self::fetch) does, require a 2xx status,
    /// and deserialize the body as JSON.
    ///
    /// # Errors
    ///
    /// - As [`fetch`](Self::fetch).
    /// - [`BadGateway`](crate::ErrorKind::BadGateway) when the answer, stored or fresh,
    ///   has a non-2xx status or a body that is not the JSON of an `O`.
    pub async fn fetch_json<O: DeserializeOwned>(&self, request: Request<Bytes>) -> Result<O> {
        super::json(self.fetch(request).await?)
    }
}

// A cache only saves time and bandwidth (RFC 9111 §1), so its own faults
// never change the answer.
impl HttpCache {
    async fn read_entry(&self, etag: &str) -> Option<Response<Bytes>> {
        let hit = match self.bucket.read(&store_key(etag)).await {
            Ok(hit) => hit?,
            Err(e) => {
                tracing::warn!("cache read for etag `{etag}` failed, treating as a miss: {e:#}");
                return None;
            }
        };
        match deserialize(&hit) {
            Ok(response) => {
                tracing::debug!("cache hit for etag `{etag}`");
                Some(response)
            }
            Err(e) => {
                tracing::warn!(
                    "cache entry for etag `{etag}` is corrupt, treating as a miss: {e:#}"
                );
                None
            }
        }
    }

    async fn write_entry(&self, etag: &str, response: &Response<Bytes>, max_age: u64) {
        tracing::debug!("caching response for etag `{etag}`");
        let bytes = match serialize(response) {
            Ok(bytes) => bytes,
            Err(e) => {
                tracing::warn!(
                    "caching response for etag `{etag}` failed, returning it uncached: {e:#}"
                );
                return;
            }
        };
        let ttl = Some(Duration::from_secs(max_age));
        if let Err(e) = self.bucket.write(&store_key(etag), &bytes, ttl).await {
            tracing::warn!(
                "caching response for etag `{etag}` failed, returning it uncached: {e:#}"
            );
        }
    }
}

/// The keyvalue key a response cached under `etag` is stored at.
///
/// The key is `http-cache.` followed by the lower-case hex SHA-256 of `etag`
/// as sent, quotes included. It is a [portable](crate::keyvalue) key whatever
/// the tag holds, so every backend accepts it.
#[must_use]
pub fn store_key(etag: &str) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";

    let mut key = String::from("http-cache.");
    for byte in Sha256::digest(etag) {
        key.push(char::from(HEX[usize::from(byte >> 4)]));
        key.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    key
}

fn storable(status: StatusCode) -> bool {
    status.is_success() && status != StatusCode::PARTIAL_CONTENT
}
