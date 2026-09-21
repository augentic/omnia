//! End-to-end tests for `wasi:http`: every scenario runs a real guest
//! component from `crates/test-programs` through the omnia runtime. Outgoing
//! scenarios are command guests against a test-owned loopback origin; the
//! incoming scenario is a handler guest driven in-process through
//! [`HttpHandler`], bypassing the socket. The guest asserts what it observes
//! across the boundary (and traps on failure); the host side asserts what
//! reached the origin and what the handler answered.

#![cfg(not(target_arch = "wasm32"))]

use std::convert::Infallible;
use std::future::ready;
use std::sync::{Arc, Mutex};

use base64ct::{Base64, Encoding};
use bytes::Bytes;
use http::header::HOST;
use http::{HeaderMap, Request, Response, StatusCode};
use http_body_util::{BodyExt as _, Full};
use hyper::body::Incoming;
use hyper::server::conn::http1;
use hyper::service::service_fn;
use omnia::ExitStatus;
use omnia_test::host::{Backends, Deployment};
use omnia_wasi_config::WasiConfig;
use omnia_wasi_http::{HttpHandler, WasiHttp};
use omnia_wasi_otel::WasiOtel;
use rcgen::{CertificateParams, ExtendedKeyUsagePurpose, KeyPair, KeyUsagePurpose};
use serde_json::{Value, json};
use tokio::net::TcpListener;
use wasmtime_wasi_http::io::TokioIo;

// Every guest program in `crates/test-programs` must have a matching test
// here; a new program without one fails to compile.
test_programs::foreach_http!();

/// The body the origin answers every request with.
const BODY: &[u8] = b"hello from origin";

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
    // Linked by hand: the guest reads the origin from `wasi:config`,
    // alongside the host under test.
    let status = Deployment::new()
        .guest("guest", wasm)
        .run(backends, |deployment| {
            deployment.host::<WasiHttp, Backends>()?;
            deployment.host::<WasiConfig, Backends>()?;
            deployment.host::<WasiOtel, Backends>()?;
            Ok(())
        })
        .await
        .expect("guest runs");
    assert_eq!(status, ExitStatus::SUCCESS, "guest `{wasm}` failed");
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

/// A base64 `Client-Cert` bundle — throwaway self-signed P-256 certificate
/// with the given extended key usage, then its PKCS#8 key — minted per run
/// so no private key is ever checked in.
fn client_cert(purpose: ExtendedKeyUsagePurpose) -> String {
    let key = KeyPair::generate().expect("key pair");
    let mut params = CertificateParams::new(Vec::<String>::new()).expect("parameters");
    params.extended_key_usages = vec![purpose];
    params.key_usages = vec![KeyUsagePurpose::DigitalSignature];
    let certificate = params.self_signed(&key).expect("certificate");
    Base64::encode_string(format!("{}{}", certificate.pem(), key.serialize_pem()).as_bytes())
}

#[tokio::test]
async fn http_outgoing_client_cert() {
    let (origin, url) = Origin::serve().await;
    let backends = Backends::defaults().await.config([
        ("ORIGIN", url),
        ("CLIENT_CERT_OK", client_cert(ExtendedKeyUsagePurpose::ClientAuth)),
        ("CLIENT_CERT_SERVER", client_cert(ExtendedKeyUsagePurpose::ServerAuth)),
    ]);

    run_guest(test_programs::HTTP_OUTGOING_CLIENT_CERT, backends).await;

    // Only the client-auth request reached the origin — the server-auth
    // bundle was refused before connecting — and the bundle itself never
    // left the host.
    let hits = origin.hits();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].path, "/mtls");
    assert!(hits[0].headers.get("client-cert").is_none(), "Client-Cert header stripped");
}

#[tokio::test]
async fn http_incoming_echo() {
    // The handler guest exports no `wasi:cli/run`: boot without driving it
    // and dispatch requests through the trigger's in-process handler.
    let runtime = Deployment::new()
        .guest("guest", test_programs::HTTP_INCOMING_ECHO)
        .boot(Backends::defaults().await, |deployment| {
            deployment.host::<WasiHttp, Backends>()?;
            deployment.host::<WasiOtel, Backends>()?;
            Ok(())
        })
        .await
        .expect("runtime boots");
    let handler = HttpHandler::new(&runtime)
        .expect("http routes consistent")
        .expect("the guest exports the http handler");

    // GET: the message is the query parameter; the transport's request id
    // header comes back as the guest saw it in its metadata.
    let request = Request::get("/echo?message=hi")
        .header(HOST, "echo.test")
        .header("x-request-id", "req-get")
        .body(Full::new(Bytes::new()))
        .expect("request");
    let response = handler.handle(request).await.expect("handled");
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(collect(response).await, json!({ "message": "hi", "request_id": "req-get" }));

    // POST: the message is the JSON body.
    let request = Request::post("/echo")
        .header(HOST, "echo.test")
        .header("x-request-id", "req-post")
        .body(Full::new(Bytes::from_static(br#"{"message":"posted"}"#)))
        .expect("request");
    let response = handler.handle(request).await.expect("handled");
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(collect(response).await, json!({ "message": "posted", "request_id": "req-post" }));

    runtime.shutdown();
}

// A trigger-served guest roots a chain of its own: baggage the handler sets
// reaches the echoer it dispatches to over `omnia-test:link/ops`.
#[tokio::test]
async fn http_baggage() {
    let runtime = Deployment::new()
        .link(["omnia-test:link/ops"])
        .guest("handler", test_programs::HTTP_BAGGAGE)
        .guest("echoer", test_programs::OTEL_BAGGAGE_ECHOER)
        .boot(Backends::defaults().await, |deployment| {
            deployment.host::<WasiHttp, Backends>()?;
            deployment.host::<WasiOtel, Backends>()?;
            Ok(())
        })
        .await
        .expect("runtime boots");
    let handler = HttpHandler::new(&runtime)
        .expect("http routes consistent")
        .expect("the handler exports the http handler");

    let request = Request::get("/baggage")
        .header(HOST, "baggage.test")
        .body(Full::new(Bytes::new()))
        .expect("request");
    let response = handler.handle(request).await.expect("handled");
    assert_eq!(response.status(), StatusCode::OK);
    let body = response.into_body().collect().await.expect("body streams").to_bytes();
    assert_eq!(body, "acme");

    runtime.shutdown();
}

/// The handler's response body, read to the end as JSON.
async fn collect(response: Response<omnia_wasi_http::OutgoingBody>) -> Value {
    let body = response.into_body().collect().await.expect("body streams").to_bytes();
    serde_json::from_slice(&body).expect("JSON body")
}
