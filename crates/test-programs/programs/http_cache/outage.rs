//! An `HttpCache` over the `broken` bucket, whose every call fails.
//!
//! The read fails, the origin is asked, and the write fails. The caller still
//! gets the origin's response on every call, so the suite counts one request
//! per call.

#![cfg(target_arch = "wasm32")]

use bytes::Bytes;
use http::header::{CACHE_CONTROL, ETAG, IF_NONE_MATCH};
use http::{HeaderValue, Request, StatusCode};
use omnia_sdk::http::cache::HttpCache;
use omnia_sdk::keyvalue;

omnia_sdk::command::export!(scenario);

async fn scenario() {
    let bucket = keyvalue::open("broken").await.expect("open the broken bucket");
    let cache = HttpCache::new(bucket);

    for _ in 1..=2 {
        let request = Request::get("https://origin.test/outage")
            .header(CACHE_CONTROL, "max-age=60")
            .header(IF_NONE_MATCH, "\"v1\"")
            .body(Bytes::new())
            .expect("a valid request");
        let response = cache.fetch(request).await.expect("the origin's response");
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.body(), b"payload".as_slice());
        assert_eq!(response.headers().get(ETAG), Some(&HeaderValue::from_static("\"v1\"")));
    }
}
