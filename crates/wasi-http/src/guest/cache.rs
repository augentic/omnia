//! Cache header parsing and cache get/put

use anyhow::{Context, Result, anyhow, bail};
use bytes::Bytes;
use http::header::{CACHE_CONTROL, IF_NONE_MATCH};
use http::{Request, Response};
use http_body::Body;
use rkyv::{Archive, Deserialize as RkyvDeserialize, Serialize as RkyvSerialize};

pub const CACHE_BUCKET: &str = "default-cache";

/// A cache instance for storing and retrieving responses.
#[derive(Debug, Default)]
pub struct Cache {
    control: Control,
    bucket: String,
}

/// Request extension used to indicate optional caching behavior.
#[derive(Clone, Debug)]
pub struct CacheOptions {
    /// Name of the key-value store bucket to use for caching.
    pub bucket_name: String,
}

impl Default for CacheOptions {
    fn default() -> Self {
        Self {
            bucket_name: CACHE_BUCKET.to_string(),
        }
    }
}

impl Cache {
    /// Create a Cache instance from the request headers, if caching is indicated.
    ///
    /// # Errors
    ///
    /// Returns an error if cache control headers are malformed.
    pub fn maybe_from(request: &Request<impl Body>) -> Result<Option<Self>> {
        let headers = request.headers();
        if headers.get(CACHE_CONTROL).is_none() {
            tracing::debug!("no Cache-Control header present");
            return Ok(None);
        }
        let control = Control::try_from(headers).context("issue parsing Cache-Control headers")?;
        let cache_opts = request
            .extensions()
            .get::<CacheOptions>()
            .map_or_else(CacheOptions::default, Clone::clone);

        Ok(Some(Self {
            bucket: cache_opts.bucket_name,
            control,
        }))
    }

    /// Get a cached response.
    ///
    /// # Errors
    ///
    /// * cache retrieval errors
    /// * deserialization errors
    pub async fn get(&self) -> Result<Option<Response<Bytes>>> {
        let ctrl = &self.control;

        if ctrl.no_cache || ctrl.no_store || ctrl.etag.is_empty() {
            tracing::debug!("cache is disabled");
            return Ok(None);
        }

        tracing::debug!("retrieving cached response with etag `{}`", &ctrl.etag);

        let cache = qwasr_wasi_keyvalue::cache::open(&self.bucket).await?;
        cache
            .get(&ctrl.etag)
            .await
            .context("retrieving cached response")?
            .map_or(Ok(None), |data| deserialize(&data).map(Some))
    }

    /// Put the response into cache.
    ///
    /// # Errors
    ///
    /// * serialization errors
    /// * cache storage errors
    pub async fn put(&self, response: &Response<Bytes>) -> Result<()> {
        let ctrl = &self.control;
        if ctrl.no_store || ctrl.etag.is_empty() || ctrl.max_age == 0 {
            return Ok(());
        }

        tracing::debug!("caching response with etag `{}`", &ctrl.etag);

        let cache = qwasr_wasi_keyvalue::cache::open(&self.bucket).await?;
        cache
            .set(&ctrl.etag, &serialize(response)?, Some(ctrl.max_age))
            .await
            .map_or_else(|e| Err(anyhow!("caching response: {e}")), |_| Ok(()))
    }

    /// Getter for etag
    #[must_use]
    pub fn etag(&self) -> String {
        self.control.etag.clone()
    }
}

#[derive(Clone, Debug, Default)]
struct Control {
    // If true, make the HTTP request and then update the cache with the
    // response.
    no_cache: bool,

    // If true, make the HTTP request and do not cache the response.
    no_store: bool,

    // Length of time to cache the response in seconds.
    max_age: u64,

    // ETag to use as the cache key, derived from the `If-None-Match` header.
    etag: String,
}

impl TryFrom<&http::HeaderMap> for Control {
    type Error = anyhow::Error;

    fn try_from(headers: &http::HeaderMap) -> Result<Self> {
        let mut control = Self::default();

        let cache_control = headers.get(CACHE_CONTROL);
        let Some(cache_control) = cache_control else {
            tracing::debug!("no Cache-Control header present");
            return Ok(control);
        };

        if cache_control.is_empty() {
            bail!("Cache-Control header is empty");
        }

        for directive in cache_control.to_str()?.split(',') {
            let directive = directive.trim().to_ascii_lowercase();
            if directive.is_empty() {
                continue;
            }

            if directive == "no-store" {
                if control.no_cache || control.max_age > 0 {
                    bail!("`no-store` cannot be combined with other cache directives");
                }
                control.no_store = true;
                continue;
            }

            if directive == "no-cache" {
                if control.no_store {
                    bail!("`no-cache` cannot be combined with `no-store`");
                }
                control.no_cache = true;
                continue;
            }

            if let Some(value) = directive.strip_prefix("max-age=") {
                if control.no_store {
                    bail!("`max-age` cannot be combined with `no-store`");
                }
                let Ok(max_age) = value.trim().parse() else {
                    bail!("`max-age` directive is malformed");
                };
                control.max_age = max_age;
            }

            // ... other directives ignored
        }

        if !control.no_store && !control.no_cache {
            let Some(etag) = headers.get(IF_NONE_MATCH) else {
                bail!(
                    "`If-None-Match` header required when using `Cache-Control: max-age` or `no-cache`"
                );
            };
            if etag.is_empty() {
                bail!("`If-None-Match` header is empty");
            }

            let etag_str = etag.to_str()?;
            if etag_str.contains(',') {
                bail!("multiple `etag` values in `If-None-Match` header are not supported");
            }
            if etag_str.starts_with("W/") {
                bail!("weak `etag` values in `If-None-Match` header are not supported");
            }
            control.etag = etag_str.to_string();
        }

        Ok(control)
    }
}

fn serialize(response: &Response<Bytes>) -> Result<Vec<u8>> {
    let ser = Serialized::try_from(response)?;
    rkyv::to_bytes::<rkyv::rancor::Error>(&ser)
        .map(|bytes| bytes.to_vec())
        .map_err(|e| anyhow!("serializing response: {e}"))
}

fn deserialize(data: &[u8]) -> Result<Response<Bytes>> {
    let ser: Serialized = rkyv::from_bytes::<Serialized, rkyv::rancor::Error>(data)
        .map_err(|e| anyhow!("deserializing cached response: {e}"))?;
    Response::<Bytes>::try_from(ser)
}

#[derive(Archive, RkyvDeserialize, RkyvSerialize)]
struct Serialized {
    status: u16,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

impl TryFrom<&Response<Bytes>> for Serialized {
    type Error = anyhow::Error;

    fn try_from(response: &Response<Bytes>) -> Result<Self> {
        Ok(Self {
            status: response.status().as_u16(),
            headers: response
                .headers()
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_str().unwrap_or_default().to_string()))
                .collect(),
            body: response.body().to_vec(),
        })
    }
}

impl TryFrom<Serialized> for Response<Bytes> {
    type Error = anyhow::Error;

    fn try_from(s: Serialized) -> Result<Self> {
        let mut response = Response::builder().status(s.status);
        for (k, v) in s.headers {
            response = response.header(k, v);
        }
        response.body(Bytes::from(s.body)).context("building response from cached data")
    }
}

#[cfg(test)]
mod tests {
    use http::HeaderMap;
    use http::header::{CACHE_CONTROL, IF_NONE_MATCH};

    use super::*;

    #[test]
    fn validates_serialization_deserialization() {
        let body = b"{\"ok\":true}";
        let response = Response::builder()
            .status(201)
            .header("content-type", "application/json")
            .header("etag", "cached")
            .header(CACHE_CONTROL, "max-age=20")
            .header(IF_NONE_MATCH, "\"label=AMP        633\"")
            .body(Bytes::from_static(body))
            .expect("should build response");

        // simulating the serialization & de-serialization that happens during cache put and get
        let serialized = serialize(&response).unwrap();
        let deserialized_response = deserialize(&serialized).unwrap();

        assert_eq!(deserialized_response.status(), response.status());
        assert_eq!(
            deserialized_response.headers().get("content-type").unwrap(),
            response.headers().get("content-type").unwrap()
        );
        assert_eq!(
            deserialized_response.headers().get("etag").unwrap(),
            response.headers().get("etag").unwrap()
        );
        assert_eq!(
            deserialized_response.headers().get(CACHE_CONTROL).unwrap(),
            response.headers().get(CACHE_CONTROL).unwrap()
        );
        assert_eq!(
            deserialized_response.headers().get(IF_NONE_MATCH).unwrap(),
            response.headers().get(IF_NONE_MATCH).unwrap()
        );
        assert_eq!(deserialized_response.body(), response.body());
    }

    #[test]
    fn returns_none_when_header_missing() {
        let headers = HeaderMap::new();
        let control = Control::try_from(&headers).expect("should parse");

        assert!(!control.no_cache);
        assert!(!control.no_store);
        assert_eq!(control.max_age, 0);
        assert!(control.etag.is_empty());
    }

    #[test]
    fn parses_max_age_with_etag() {
        let mut headers = HeaderMap::new();
        headers.append(CACHE_CONTROL, "max-age=120".parse().unwrap());
        headers.append(IF_NONE_MATCH, "\"strong-etag\"".parse().unwrap());

        let control = Control::try_from(&headers).expect("should parse");

        assert!(!control.no_store);
        assert_eq!(control.max_age, 120);
        assert_eq!(control.etag, "\"strong-etag\"");
    }

    #[test]
    fn requibe_etag_when_store_enabled() {
        let mut headers = HeaderMap::new();
        headers.append(CACHE_CONTROL, "no-cache".parse().unwrap());

        let Err(_) = Control::try_from(&headers) else {
            panic!("expected missing etag error");
        };
    }

    #[test]
    fn rejects_conflicting_directives() {
        let mut headers = HeaderMap::new();
        headers.append(CACHE_CONTROL, "no-store, no-cache, max-age=10".parse().unwrap());
        headers.append(IF_NONE_MATCH, "\"etag\"".parse().unwrap());

        let Err(_) = Control::try_from(&headers) else {
            panic!("expected conflicting directives error");
        };
    }

    #[test]
    fn rejects_weak_etag_value() {
        let mut headers = HeaderMap::new();
        headers.append(CACHE_CONTROL, "no-cache".parse().unwrap());
        headers.append(IF_NONE_MATCH, "W/\"weak-etag\"".parse().unwrap());

        let Err(_) = Control::try_from(&headers) else {
            panic!("expected weak etag rejection");
        };
    }

    #[test]
    fn rejects_multiple_etag_values() {
        let mut headers = HeaderMap::new();
        headers.append(CACHE_CONTROL, "no-cache".parse().unwrap());
        headers.append(IF_NONE_MATCH, "\"etag1\", \"etag2\"".parse().unwrap());

        let Err(_) = Control::try_from(&headers) else {
            panic!("expected multiple etag values rejection");
        };
    }
}
