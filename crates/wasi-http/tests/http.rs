//! End-to-end tests for outgoing `wasi:http`: every scenario runs a real
//! guest component from `crates/test-programs` through the omnia runtime
//! against a test-owned loopback origin. The guest asserts what it observes
//! across the boundary (and traps on failure); the host side asserts what
//! reached the origin and what the guest-side cache persisted.

#![cfg(not(target_arch = "wasm32"))]

use std::convert::Infallible;
use std::future::ready;
use std::sync::{Arc, Mutex};

use bytes::Bytes;
use http::header::IF_NONE_MATCH;
use http::{HeaderMap, Request, Response};
use http_body_util::Full;
use hyper::body::Incoming;
use hyper::server::conn::http1;
use hyper::service::service_fn;
use omnia::ExitStatus;
use omnia_test::host::{Backends, Deployment};
use omnia_wasi_config::WasiConfig;
use omnia_wasi_http::WasiHttp;
use omnia_wasi_keyvalue::{WasiKeyValue, WasiKeyValueCtx as _};
use omnia_wasi_otel::WasiOtel;
use serde_json::Value;
use tokio::net::TcpListener;
use wasmtime_wasi_http::io::TokioIo;

// Every guest program in `crates/test-programs` must have a matching test
// here; a new program without one fails to compile.
test_programs::foreach_http!();

/// The body the origin answers every request with.
const BODY: &[u8] = b"hello from origin";

/// The bucket `omnia_wasi_http`'s guest-side cache opens by default.
const CACHE_BUCKET: &str = "default-cache";

/// One request as the origin saw it.
#[derive(Clone, Debug)]
struct Hit {
    path: String,
    headers: HeaderMap,
}

/// A loopback origin answering every request with `200`, a marker header,
/// and [`BODY`] — honouring nothing conditional — and recording each hit.
#[derive(Clone, Debug, Default)]
struct Origin {
    hits: Arc<Mutex<Vec<Hit>>>,
}

impl Origin {
    /// Serves a fresh origin on an ephemeral port for the life of the test
    /// runtime; returns it with its base URL.
    async fn serve() -> (Self, String) {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("binding loopback");
        let addr = listener.local_addr().expect("listener address");
        let origin = Self::default();

        let accepting = origin.clone();
        tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                let origin = accepting.clone();
                tokio::spawn(async move {
                    let service = service_fn(move |request| {
                        ready(Ok::<_, Infallible>(origin.answer(&request)))
                    });
                    let _ =
                        http1::Builder::new().serve_connection(TokioIo::new(stream), service).await;
                });
            }
        });

        (origin, format!("http://{addr}"))
    }

    fn answer(&self, request: &Request<Incoming>) -> Response<Full<Bytes>> {
        self.hits.lock().expect("hits lock").push(Hit {
            path: request.uri().path().to_owned(),
            headers: request.headers().clone(),
        });
        Response::builder()
            .header("x-mock", "origin")
            .body(Full::new(Bytes::from_static(BODY)))
            .expect("origin response")
    }

    fn hits(&self) -> Vec<Hit> {
        self.hits.lock().expect("hits lock").clone()
    }
}

/// Run one guest program against `backends`, requiring a clean exit.
async fn run_guest(wasm: &str, backends: Backends) {
    // Linked by hand: the guest reads the origin from `wasi:config` and its
    // cache lives in `wasi:keyvalue`, alongside the host under test.
    let status = Deployment::new()
        .guest("guest", wasm)
        .run(backends, |deployment| {
            deployment.host::<WasiHttp, Backends>()?;
            deployment.host::<WasiConfig, Backends>()?;
            deployment.host::<WasiKeyValue, Backends>()?;
            deployment.host::<WasiOtel, Backends>()?;
            Ok(())
        })
        .await
        .expect("guest runs");
    assert_eq!(status, ExitStatus::SUCCESS, "guest `{wasm}` failed");
}

/// A JSON array of byte values as bytes.
fn bytes(value: &Value) -> Vec<u8> {
    value
        .as_array()
        .expect("byte array")
        .iter()
        .map(|byte| u8::try_from(byte.as_u64().expect("byte")).expect("byte range"))
        .collect()
}

#[tokio::test]
async fn http_outgoing_get() {
    let (origin, url) = Origin::serve().await;
    let backends = Backends::defaults().await.config([("ORIGIN", url)]);

    run_guest(test_programs::HTTP_OUTGOING_GET, backends).await;

    // Wire fidelity: one request reached the origin, at the guest's path,
    // carrying the guest's header.
    let hits = origin.hits();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].path, "/resource");
    assert_eq!(hits[0].headers.get("x-probe").and_then(|v| v.to_str().ok()), Some("1"));
}

#[tokio::test]
async fn http_cache_hit() {
    let (origin, url) = Origin::serve().await;
    let backends = Backends::defaults().await.config([("ORIGIN", url)]);

    run_guest(test_programs::HTTP_CACHE_HIT, backends.clone()).await;

    // The second GET never left the guest, and the one that did carried no
    // conditional header: the guest-side cache owns those semantics.
    let hits = origin.hits();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].path, "/cached");
    assert!(hits[0].headers.get(IF_NONE_MATCH).is_none(), "If-None-Match is not forwarded");

    // Persisted state: the cache bucket holds the origin's response under
    // the etag, wrapped in the keyvalue `Cacheable` envelope.
    let bucket = backends.keyvalue.open_bucket(CACHE_BUCKET.to_owned()).await.expect("bucket");
    assert_eq!(bucket.keys().await.expect("keys"), ["\"v1\""]);
    let entry = bucket.get("\"v1\"".to_owned()).await.expect("get").expect("cached entry");
    let envelope: Value = serde_json::from_slice(&entry).expect("Cacheable JSON");
    let cached: Value =
        serde_json::from_slice(&bytes(&envelope["value"])).expect("serialized response");
    assert_eq!(cached["status"], 200);
    assert_eq!(bytes(&cached["body"]), BODY);
}
