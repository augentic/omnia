//! The `HttpCache` header contract over the `default` bucket.
//!
//! The guest asserts each response and refusal it sees. Every scenario
//! fetches its own path and sends its own etag, so the suite counts the
//! requests each one sent and reads back what each one stored.

#![cfg(target_arch = "wasm32")]

use bytes::Bytes;
use http::header::{CACHE_CONTROL, CONTENT_DISPOSITION, CONTENT_TYPE, ETAG, IF_NONE_MATCH};
use http::{HeaderName, HeaderValue, Request, Response, StatusCode};
use omnia_sdk::http::cache::{HttpCache, store_key};
use omnia_sdk::keyvalue::{self, Bucket};

const ORIGIN_ETAG: &str = "\"origin\"";
const PAYLOAD: &[u8] = b"payload";
const DISPOSITION: &[u8] = b"attachment; filename=\"r\xE9sum\xE9.pdf\"";

omnia_sdk::command::export!(scenario);

async fn scenario() {
    let client = Client::open().await;

    miss_then_hit(&client).await;
    no_cache_max_age(&client).await;
    multiline_cache_control(&client).await;
    no_cache(&client).await;
    zero_max_age(&client).await;
    no_store(&client).await;
    server_error(&client).await;
    partial_content(&client).await;
    corrupt_entry(&client).await;
    namespaced_etag(&client).await;
    absent_cache_control(&client).await;
    unknown_directives(&client).await;
    bad_directives(&client).await;
}

async fn miss_then_hit(client: &Client) {
    let headers = [(CACHE_CONTROL, "max-age=60"), (IF_NONE_MATCH, "\"v1\"")];
    let miss = client.fetch("hit", &headers).await;
    assert_served(&miss, StatusCode::OK, "\"v1\"");

    // the hit is the stored copy, with every header the origin sent intact
    let hit = client.fetch("hit", &headers).await;
    assert_served(&hit, StatusCode::OK, "\"v1\"");
    assert_eq!(header(&hit, CONTENT_TYPE), Some(b"text/plain".as_slice()));
    assert_eq!(header(&hit, CONTENT_DISPOSITION), Some(DISPOSITION));
}

async fn no_cache_max_age(client: &Client) {
    let etag = "\"refresh\"";
    let headers = [(CACHE_CONTROL, "no-cache, max-age=60"), (IF_NONE_MATCH, etag)];

    // a stale entry is bypassed and then replaced, every round
    for _ in 1..=2 {
        client.plant(etag, b"stale").await;
        let response = client.fetch("refresh", &headers).await;
        assert_served(&response, StatusCode::OK, etag);
    }
}

async fn multiline_cache_control(client: &Client) {
    let etag = "\"multiline\"";
    client.plant(etag, b"stale").await;

    // two lines are one list (RFC 9110 §5.3): a `no-cache, max-age=60`
    // refresh, not a bare `no-cache` bypass
    let headers =
        [(CACHE_CONTROL, "no-cache"), (CACHE_CONTROL, "max-age=60"), (IF_NONE_MATCH, etag)];
    let response = client.fetch("multiline", &headers).await;
    assert_served(&response, StatusCode::OK, etag);

    // and a `no-store` on a later line conflicts
    let headers =
        [(CACHE_CONTROL, "max-age=60"), (CACHE_CONTROL, "no-store"), (IF_NONE_MATCH, etag)];
    client.refuse("multiline", &headers).await;
}

async fn no_cache(client: &Client) {
    let etag = "\"bypass\"";
    client.plant(etag, b"existing").await;

    // RFC 9111 §5.2.1.4: the stored copy is not served unvalidated, and with
    // no `max-age` there is no lifetime to store the response under
    let headers = [(CACHE_CONTROL, "no-cache"), (IF_NONE_MATCH, etag)];
    let response = client.fetch("bypass", &headers).await;
    assert_served(&response, StatusCode::OK, etag);
}

async fn zero_max_age(client: &Client) {
    let etag = "\"revalidate\"";
    client.plant(etag, b"stale").await;

    // RFC 9111 §5.2.1.1: `max-age=0` accepts no stored response, and gives
    // the fresh copy no lifetime to be stored under
    let headers = [(CACHE_CONTROL, "max-age=0"), (IF_NONE_MATCH, etag)];
    let response = client.fetch("revalidate", &headers).await;
    assert_served(&response, StatusCode::OK, etag);
}

async fn no_store(client: &Client) {
    // `no-store` records no etag, and RFC 9110 §8.8.3 admits no empty
    // entity-tag, so the origin's `ETag` stands
    let headers = [(CACHE_CONTROL, "no-store"), (IF_NONE_MATCH, "\"no-store\"")];
    let response = client.fetch("no-store", &headers).await;
    assert_served(&response, StatusCode::OK, ORIGIN_ETAG);
}

async fn server_error(client: &Client) {
    let headers = [(CACHE_CONTROL, "max-age=60"), (IF_NONE_MATCH, "\"error\"")];
    let response = client.fetch("error", &headers).await;
    assert_served(&response, StatusCode::INTERNAL_SERVER_ERROR, "\"error\"");
}

async fn partial_content(client: &Client) {
    // RFC 9111 §3: a cache that does not combine ranges returns a 206 but
    // never keys it as the whole resource
    let headers = [(CACHE_CONTROL, "max-age=60"), (IF_NONE_MATCH, "\"partial\"")];
    let response = client.fetch("partial", &headers).await;
    assert_served(&response, StatusCode::PARTIAL_CONTENT, "\"partial\"");
}

async fn corrupt_entry(client: &Client) {
    let etag = "\"corrupt\"";
    client.plant(etag, b"not an envelope").await;

    // the corrupt entry is a miss, and the origin's response replaces it
    let headers = [(CACHE_CONTROL, "max-age=60"), (IF_NONE_MATCH, etag)];
    for _ in 1..=2 {
        let response = client.fetch("corrupt", &headers).await;
        assert_served(&response, StatusCode::OK, etag);
    }
}

async fn namespaced_etag(client: &Client) {
    // the quotes, `:`, `=`, and `&` are no portable key's, and a store that
    // refuses such a key still serves the miss's entry as a hit
    let etag = "\"fleet:route=1&stop=2\"";
    let headers = [(CACHE_CONTROL, "max-age=60"), (IF_NONE_MATCH, etag)];
    for _ in 1..=2 {
        let response = client.fetch("namespaced", &headers).await;
        assert_served(&response, StatusCode::OK, etag);
    }
}

async fn absent_cache_control(client: &Client) {
    let response = client.fetch("plain", &[(IF_NONE_MATCH, "\"plain\"")]).await;
    assert_served(&response, StatusCode::OK, ORIGIN_ETAG);
}

async fn unknown_directives(client: &Client) {
    // RFC 9111 §5.2.3: no directive here is one this cache acts on, so the
    // request leaves as sent and the origin's `ETag` stands
    let headers = [(CACHE_CONTROL, "no-transform"), (IF_NONE_MATCH, "\"unknown\"")];
    let response = client.fetch("unknown", &headers).await;
    assert_served(&response, StatusCode::OK, ORIGIN_ETAG);

    // nor is a missing etag an error
    let response = client.fetch("unknown", &[(CACHE_CONTROL, "no-transform")]).await;
    assert_served(&response, StatusCode::OK, ORIGIN_ETAG);
}

async fn bad_directives(client: &Client) {
    for headers in [
        vec![(CACHE_CONTROL, "max-age=60")],
        vec![(CACHE_CONTROL, "max-age=60, max-age=0"), (IF_NONE_MATCH, "\"bad\"")],
        vec![(CACHE_CONTROL, "no-store, max-age=0"), (IF_NONE_MATCH, "\"bad\"")],
        vec![(CACHE_CONTROL, "max-age=60"), (IF_NONE_MATCH, "W/\"weak\"")],
        vec![(CACHE_CONTROL, "max-age=60"), (IF_NONE_MATCH, "\"a\", \"b\"")],
        vec![(CACHE_CONTROL, "max-age=60"), (IF_NONE_MATCH, "*")],
        vec![(CACHE_CONTROL, "max-age=60"), (IF_NONE_MATCH, "unquoted")],
    ] {
        client.refuse("bad", &headers).await;
    }
}

// the cache under test, and a second handle on its bucket to plant entries
struct Client {
    cache: HttpCache,
    bucket: Bucket,
}

impl Client {
    async fn open() -> Self {
        let cache = HttpCache::new(Self::bucket().await);
        Self {
            cache,
            bucket: Self::bucket().await,
        }
    }

    async fn bucket() -> Bucket {
        keyvalue::open(keyvalue::DEFAULT).await.expect("open the default bucket")
    }

    async fn fetch(&self, path: &str, headers: &[(HeaderName, &str)]) -> Response<Bytes> {
        let response = self.cache.fetch(request(path, headers)).await;
        response.unwrap_or_else(|err| panic!("fetching `{path}` with {headers:?}: {err}"))
    }

    async fn refuse(&self, path: &str, headers: &[(HeaderName, &str)]) {
        let Err(err) = self.cache.fetch(request(path, headers)).await else {
            panic!("fetching `{path}` with {headers:?} was not refused");
        };
        assert_eq!(err.code(), "invalid", "fetching `{path}` with {headers:?}: {err}");
    }

    async fn plant(&self, etag: &str, value: &[u8]) {
        self.bucket.write(&store_key(etag), value, None).await.expect("plant an entry");
    }
}

fn request(path: &str, headers: &[(HeaderName, &str)]) -> Request<Bytes> {
    let mut request = Request::get(format!("https://origin.test/{path}"));
    for (name, value) in headers {
        request = request.header(name, *value);
    }
    request.body(Bytes::new()).expect("a valid request")
}

#[track_caller]
fn assert_served(response: &Response<Bytes>, status: StatusCode, etag: &str) {
    assert_eq!(response.status(), status);
    assert_eq!(response.body(), PAYLOAD);
    let stamped = response.headers().get(ETAG).and_then(|value| value.to_str().ok());
    assert_eq!(stamped, Some(etag));
}

fn header(response: &Response<Bytes>, name: HeaderName) -> Option<&[u8]> {
    response.headers().get(name).map(HeaderValue::as_bytes)
}
