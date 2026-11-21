//! End-to-end tests for `HttpCache`: each program under
//! `crates/test-programs/programs/http_cache` runs as a real guest component
//! through the omnia runtime. It fetches from a [`MatchedHttp`] origin and
//! caches in a keyvalue store that, like NATS, refuses a key that is not
//! portable. The guest asserts the responses and refusals it sees; this side
//! asserts what reached the origin and what the bucket kept.

#![cfg(not(target_arch = "wasm32"))]

use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use http::header::{CONTENT_DISPOSITION, CONTENT_TYPE, ETAG, IF_NONE_MATCH};
use http::{HeaderValue, Method, Response, StatusCode};
use omnia::{ExitStatus, Provides};
use omnia_sdk::http::cache::store_key;
use omnia_test::host::{Backends, Deployment, MatchedHttp};
use omnia_wasi_http::WasiHttp;
use omnia_wasi_keyvalue::{
    Bucket, Entry, Error, FutureResult, KeyValueClient, KeyValueDefault, Lifetime, Precondition,
    Revision, WasiKeyValue,
};
use omnia_wasi_otel::WasiOtel;
use serde_json::{Value, json};

test_programs::foreach_http_cache!();

#[tokio::test]
async fn http_cache_headers() {
    let origin = origin(&[
        ("hit", StatusCode::OK),
        ("refresh", StatusCode::OK),
        ("multiline", StatusCode::OK),
        ("bypass", StatusCode::OK),
        ("revalidate", StatusCode::OK),
        ("no-store", StatusCode::OK),
        ("error", StatusCode::INTERNAL_SERVER_ERROR),
        ("partial", StatusCode::PARTIAL_CONTENT),
        ("corrupt", StatusCode::OK),
        ("namespaced", StatusCode::OK),
        ("plain", StatusCode::OK),
        ("unknown", StatusCode::OK),
    ])
    .on(
        Method::GET,
        "https://origin.test/json",
        Response::new(Bytes::from_static(br#"{"payload":true}"#)),
    );
    let backends = Backends::defaults().await;
    let store = Strict(backends.keyvalue.clone());
    let backends = backends.keyvalue(store).http(origin.clone());

    run_guest(test_programs::HTTP_CACHE_HEADERS, backends.clone()).await;

    // one request per miss, and none for a hit or a refused directive
    let requests = origin.requests();
    let paths: Vec<_> = requests.iter().map(|request| request.uri.path()).collect();
    assert_eq!(
        paths,
        [
            "/hit",
            "/refresh",
            "/refresh",
            "/multiline",
            "/bypass",
            "/revalidate",
            "/no-store",
            "/error",
            "/partial",
            "/corrupt",
            "/namespaced",
            "/plain",
            "/unknown",
            "/unknown",
            "/json",
        ]
    );

    // the cache owns conditional semantics, so the origin never answers a 304
    // in place of the resource; only a request it does not act on goes as sent
    let conditional: Vec<_> = requests
        .iter()
        .filter_map(|request| {
            let etag = request.headers.get(IF_NONE_MATCH)?;
            Some((request.uri.path(), etag.as_bytes()))
        })
        .collect();
    assert_eq!(
        conditional,
        [("/plain", b"\"plain\"".as_slice()), ("/unknown", b"\"unknown\"".as_slice())]
    );

    // what the bucket holds under an etag's key, and the envelope the crate
    // writes there as JSON
    let stored = async |etag: &str| {
        let stored = backends.state(&store_key(etag)).await;
        stored.map(|bytes| String::from_utf8(bytes).expect("a UTF-8 entry"))
    };
    let envelope = async |etag: &str| -> Value {
        let stored = stored(etag).await.unwrap_or_else(|| panic!("no entry for {etag}"));
        serde_json::from_str(&stored).expect("an envelope is JSON")
    };

    // the miss stored the stamped response, every header value as bytes
    let miss = envelope("\"v1\"").await;
    assert_eq!(miss["status"], 200);
    assert_eq!(miss["body"], json!(b"payload"));
    let headers = miss["headers"].as_array().expect("an envelope's headers");
    assert!(headers.contains(&json!(["etag", b"\"v1\""])), "{headers:?}");

    // a refresh over one line or two and a corrupt entry hold the origin's
    // response, and a namespaced etag is stored under its portable key
    for etag in ["\"refresh\"", "\"multiline\"", "\"corrupt\"", "\"fleet:route=1&stop=2\""] {
        assert_eq!(envelope(etag).await["status"], 200, "{etag}");
    }

    // a bypass and a revalidation leave the entry they found
    assert_eq!(stored("\"bypass\"").await.as_deref(), Some("existing"));
    assert_eq!(stored("\"revalidate\"").await.as_deref(), Some("stale"));

    for etag in ["\"no-store\"", "\"error\"", "\"partial\"", "\"plain\"", "\"unknown\"", "\"bad\""]
    {
        assert_eq!(stored(etag).await, None, "{etag}");
    }
}

#[tokio::test]
async fn http_cache_outage() {
    let origin = origin(&[("outage", StatusCode::OK)]);
    let backends = Backends::defaults().await;
    let store = Strict(backends.keyvalue.clone());
    let backends = backends.keyvalue(store).http(origin.clone());

    run_guest(test_programs::HTTP_CACHE_OUTAGE, backends).await;

    let paths: Vec<_> =
        origin.requests().iter().map(|request| request.uri.path().to_owned()).collect();
    assert_eq!(paths, ["/outage", "/outage"]);
}

async fn run_guest<B>(wasm: &str, backends: B)
where
    B: Provides<WasiHttp> + Provides<WasiKeyValue> + Provides<WasiOtel>,
{
    let status = Deployment::new()
        .guest("guest", wasm)
        .run(backends, |deployment| {
            deployment.host::<WasiHttp>()?;
            deployment.host::<WasiKeyValue>()?;
            deployment.host::<WasiOtel>()?;
            Ok(())
        })
        .await
        .expect("guest runs");
    assert_eq!(status, ExitStatus::SUCCESS, "guest `{wasm}` failed");
}

// an origin answering `GET https://origin.test/<path>` with its status, its
// own `ETag` so stamping shows, and a header value outside ASCII
fn origin(paths: &[(&str, StatusCode)]) -> MatchedHttp {
    paths.iter().fold(MatchedHttp::default(), |origin, (path, status)| {
        let mut response = Response::new(Bytes::from_static(b"payload"));
        *response.status_mut() = *status;
        let headers = response.headers_mut();
        headers.insert(ETAG, HeaderValue::from_static("\"origin\""));
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("text/plain"));
        let disposition = HeaderValue::from_bytes(b"attachment; filename=\"r\xE9sum\xE9.pdf\"");
        headers.insert(CONTENT_DISPOSITION, disposition.expect("an obs-text header value"));
        origin.on(Method::GET, format!("https://origin.test/{path}"), response)
    })
}

// a keyvalue store that refuses a key that is not portable, and whose
// `broken` bucket fails every call
#[derive(Clone, Debug)]
struct Strict(KeyValueDefault);

impl KeyValueClient for Strict {
    fn open_bucket(&self, identifier: String) -> FutureResult<Arc<dyn Bucket>> {
        if identifier == "broken" {
            return Box::pin(async { Ok(Arc::new(Broken) as Arc<dyn Bucket>) });
        }
        let opening = self.0.open_bucket(identifier);
        Box::pin(async move { Ok(Arc::new(Portable(opening.await?)) as Arc<dyn Bucket>) })
    }
}

#[derive(Debug)]
struct Portable(Arc<dyn Bucket>);

impl Bucket for Portable {
    fn get(&self, key: String) -> FutureResult<Option<Entry>> {
        refused(&key).unwrap_or_else(|| self.0.get(key))
    }

    fn set(
        &self, key: String, value: Vec<u8>, lifetime: Lifetime, when: Option<Precondition>,
    ) -> FutureResult<Result<Revision, Option<Entry>>> {
        refused(&key).unwrap_or_else(|| self.0.set(key, value, lifetime, when))
    }

    fn delete(
        &self, key: String, when: Option<Revision>,
    ) -> FutureResult<Result<(), Option<Entry>>> {
        refused(&key).unwrap_or_else(|| self.0.delete(key, when))
    }

    fn exists(&self, key: String) -> FutureResult<bool> {
        refused(&key).unwrap_or_else(|| self.0.exists(key))
    }

    fn keys(&self) -> FutureResult<Vec<String>> {
        self.0.keys()
    }

    fn increment(&self, key: String, delta: i64, ttl: Option<Duration>) -> FutureResult<i64> {
        refused(&key).unwrap_or_else(|| self.0.increment(key, delta, ttl))
    }
}

fn refused<T: Send + 'static>(key: &str) -> Option<FutureResult<T>> {
    let portable = key.len() <= 255
        && key.split('.').all(|segment| {
            !segment.is_empty()
                && segment
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
        });
    if portable {
        return None;
    }
    let error = Error::Invalid(format!("`{key}` is not a portable key"));
    Some(Box::pin(async move { Err(error.into()) }))
}

#[derive(Debug)]
struct Broken;

impl Broken {
    fn outage<T: Send + 'static>() -> FutureResult<T> {
        Box::pin(async { Err(Error::Unavailable("the suite's store is down".to_owned()).into()) })
    }
}

impl Bucket for Broken {
    fn get(&self, _key: String) -> FutureResult<Option<Entry>> {
        Self::outage()
    }

    fn set(
        &self, _key: String, _value: Vec<u8>, _lifetime: Lifetime, _when: Option<Precondition>,
    ) -> FutureResult<Result<Revision, Option<Entry>>> {
        Self::outage()
    }

    fn delete(
        &self, _key: String, _when: Option<Revision>,
    ) -> FutureResult<Result<(), Option<Entry>>> {
        Self::outage()
    }

    fn exists(&self, _key: String) -> FutureResult<bool> {
        Self::outage()
    }

    fn keys(&self) -> FutureResult<Vec<String>> {
        Self::outage()
    }

    fn increment(&self, _key: String, _delta: i64, _ttl: Option<Duration>) -> FutureResult<i64> {
        Self::outage()
    }
}
