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
use std::sync::atomic::{AtomicUsize, Ordering};
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
use rustls::pki_types::PrivateKeyDer;
use rustls::{ServerConfig, ServerConnection};
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::{TcpListener, TcpStream};
use wasmtime_wasi_http::io::TokioIo;

// Every guest program in `crates/test-programs` must have a matching test
// here; a new program without one fails to compile.
test_programs::foreach_http!();

const BODY: &[u8] = b"hello from origin";

// one request as the origin saw it
#[derive(Clone, Debug)]
struct Hit {
    path: String,
    headers: HeaderMap,
}

// a loopback origin answering every request with `200`, a marker header and
// `BODY`, honouring nothing conditional, and recording each hit
#[derive(Clone, Debug, Default)]
struct Origin {
    hits: Arc<Mutex<Vec<Hit>>>,
}

impl Origin {
    // serves on an ephemeral port for the life of the test runtime; returns the base url
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

async fn run_guest(wasm: &str, backends: Backends) {
    // linked by hand: the guest reads the origin from `wasi:config`
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

    // wire fidelity: one request, at the guest's path, carrying the guest's header
    let hits = origin.hits();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].path, "/resource");
    assert_eq!(hits[0].headers.get("x-probe").and_then(|v| v.to_str().ok()), Some("1"));
}

// a throwaway self-signed certificate then its PKCS#8 key, minted per run so
// no private key is ever checked in
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

    // only the client-auth request reached the origin, and the bundle never left the host
    let hits = origin.hits();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].path, "/mtls");
    assert!(hits[0].headers.get("client-cert").is_none(), "Client-Cert header stripped");
}

// a loopback TLS origin presenting a self-signed certificate no root trusts,
// counting the handshakes it is offered; none completes, so it serves nothing
#[derive(Clone, Debug, Default)]
struct UntrustedOrigin {
    handshakes: Arc<AtomicUsize>,
}

impl UntrustedOrigin {
    // each handshake blocks a worker thread, since the client walks away
    // mid-handshake; the accept loop goes with the test runtime
    async fn serve() -> (Self, String) {
        let key = KeyPair::generate().expect("key pair");
        let params = CertificateParams::new(vec!["127.0.0.1".to_owned()]).expect("parameters");
        let certificate = params.self_signed(&key).expect("certificate");
        let provider = Arc::new(rustls::crypto::ring::default_provider());
        let config = ServerConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()
            .expect("protocol versions")
            .with_no_client_auth()
            .with_single_cert(
                vec![certificate.der().clone()],
                PrivateKeyDer::Pkcs8(key.serialize_der().into()),
            )
            .expect("server config");
        let config = Arc::new(config);

        let listener = TcpListener::bind("127.0.0.1:0").await.expect("binding loopback");
        let addr = listener.local_addr().expect("listener address");
        let origin = Self::default();

        let accepting = origin.clone();
        tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                accepting.handshakes.fetch_add(1, Ordering::SeqCst);
                let config = Arc::clone(&config);
                tokio::task::spawn_blocking(move || {
                    let mut stream = stream.into_std().expect("std stream");
                    stream.set_nonblocking(false).expect("blocking stream");
                    let mut connection = ServerConnection::new(config).expect("server connection");
                    let _ = connection.complete_io(&mut stream);
                });
            }
        });

        (origin, format!("https://{addr}"))
    }

    fn handshakes(&self) -> usize {
        self.handshakes.load(Ordering::SeqCst)
    }
}

#[tokio::test]
async fn http_outgoing_tls() {
    let (origin, url) = UntrustedOrigin::serve().await;
    let backends = Backends::defaults().await.config([("ORIGIN", url)]);

    run_guest(test_programs::HTTP_OUTGOING_TLS, backends).await;

    // the code came out of a real handshake the origin saw, not a failure before one
    assert_eq!(origin.handshakes(), 1);
}

// a loopback origin that reads a request's head and then stalls: `/head` is
// never answered, `/body` is answered with a body that stops short of its
// declared length; either way the connection is held until the client hangs up
#[derive(Clone, Debug, Default)]
struct StallingOrigin {
    accepted: Arc<AtomicUsize>,
}

impl StallingOrigin {
    async fn serve() -> (Self, String) {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("binding loopback");
        let addr = listener.local_addr().expect("listener address");
        let origin = Self::default();

        let accepting = origin.clone();
        tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                accepting.accepted.fetch_add(1, Ordering::SeqCst);
                tokio::spawn(Self::stall(stream));
            }
        });

        (origin, format!("http://{addr}"))
    }

    async fn stall(mut stream: TcpStream) {
        let mut head = Vec::new();
        let mut chunk = [0u8; 1024];
        while !head.windows(4).any(|bytes| bytes == b"\r\n\r\n") {
            match stream.read(&mut chunk).await {
                Ok(0) | Err(_) => return,
                Ok(n) => head.extend_from_slice(&chunk[..n]),
            }
        }

        if head.starts_with(b"GET /body ") {
            let _ =
                stream.write_all(b"HTTP/1.1 200 OK\r\ncontent-length: 32\r\n\r\n12345678").await;
        }

        // the client hanging up is the only way out
        while matches!(stream.read(&mut chunk).await, Ok(n) if n > 0) {}
    }

    fn accepted(&self) -> usize {
        self.accepted.load(Ordering::SeqCst)
    }
}

#[tokio::test]
async fn http_outgoing_timeout() {
    let (origin, url) = StallingOrigin::serve().await;
    let backends = Backends::defaults().await.config([("ORIGIN", url)]);

    run_guest(test_programs::HTTP_OUTGOING_TIMEOUT, backends).await;

    // both timeouts fired on a connection the origin held, not before one was made
    assert_eq!(origin.accepted(), 2);
}

#[tokio::test]
async fn http_incoming_echo() {
    // no `wasi:cli/run` to drive: boot, then dispatch through the in-process handler
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

    // get: the message is the query parameter; the request id comes back from the metadata
    let request = Request::get("/echo?message=hi")
        .header(HOST, "echo.test")
        .header("x-request-id", "req-get")
        .body(Full::new(Bytes::new()))
        .expect("request");
    let response = handler.handle(request).await.expect("handled");
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(collect(response).await, json!({ "message": "hi", "request_id": "req-get" }));

    // post: the message is the json body
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

async fn collect(response: Response<omnia_wasi_http::OutgoingBody>) -> Value {
    let body = response.into_body().collect().await.expect("body streams").to_bytes();
    serde_json::from_slice(&body).expect("JSON body")
}
