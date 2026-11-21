//! End-to-end tests for `wasi:http`: every scenario runs a real guest
//! component from `crates/test-programs` through the omnia runtime. Outgoing
//! scenarios are command guests against a test-owned loopback origin; the
//! incoming scenario is a handler guest driven in-process through
//! [`HttpHandler`], bypassing the socket. The guest asserts what it observes
//! across the boundary (and traps on failure); the host side asserts what
//! reached the origin and what the handler answered.

#![cfg(not(target_arch = "wasm32"))]

use std::convert::Infallible;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use base64ct::{Base64, Encoding};
use bytes::Bytes;
use futures::StreamExt as _;
use futures::channel::mpsc::UnboundedSender;
use http::header::{AUTHORIZATION, CONTENT_TYPE, HOST, LOCATION, RETRY_AFTER};
use http::{HeaderMap, Method, Request, Response, StatusCode};
use http_body_util::{BodyExt as _, Full, StreamBody};
use hyper::body::{Body, Frame, Incoming};
use hyper::server::conn::http1;
use hyper::service::service_fn;
use omnia::{ExitStatus, Grants, GuestEntry, Runtime, RuntimeOptions};
use omnia_test::host::{Backends, Deployment};
use omnia_wasi_config::WasiConfig;
use omnia_wasi_http::{HttpHandler, ServerOptions, WasiHttp};
use omnia_wasi_otel::WasiOtel;
use rcgen::{
    BasicConstraints, CertificateParams, ExtendedKeyUsagePurpose, IsCa, Issuer, KeyPair,
    KeyUsagePurpose, date_time_ymd,
};
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
    method: Method,
    path: String,
    headers: HeaderMap,
    body: Bytes,
}

// a loopback origin answering every request with `200`, a marker header, and
// the request's own body where it has one or `BODY` where not, honouring
// nothing conditional, and recording each hit; `/redirect?to=<url>` alone is
// answered `303` to `<url>`, the status a client follows whatever the body
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
                        let origin = origin.clone();
                        async move { Ok::<_, Infallible>(origin.answer(request).await) }
                    });
                    let _ =
                        http1::Builder::new().serve_connection(TokioIo::new(stream), service).await;
                });
            }
        });

        (origin, format!("http://{addr}"))
    }

    async fn answer(&self, request: Request<Incoming>) -> Response<Full<Bytes>> {
        let (parts, body) = request.into_parts();
        let body = body.collect().await.expect("the request body streams").to_bytes();
        let location = match (parts.uri.path(), parts.uri.query()) {
            ("/redirect", Some(query)) => query.strip_prefix("to=").map(str::to_owned),
            _ => None,
        };
        self.hits.lock().expect("hits lock").push(Hit {
            method: parts.method,
            path: parts.uri.path().to_owned(),
            headers: parts.headers,
            body: body.clone(),
        });
        if let Some(location) = location {
            return Response::builder()
                .status(StatusCode::SEE_OTHER)
                .header(LOCATION, location)
                .body(Full::default())
                .expect("origin redirect");
        }
        let reply = if body.is_empty() { Bytes::from_static(BODY) } else { body };
        Response::builder()
            .header("x-mock", "origin")
            .body(Full::new(reply))
            .expect("origin response")
    }

    fn hits(&self) -> Vec<Hit> {
        self.hits.lock().expect("hits lock").clone()
    }
}

async fn run_guest(wasm: &str, backends: Backends) {
    run_deployment(Deployment::new().guest("guest", wasm), backends).await;
}

async fn run_deployment(deployment: Deployment, backends: Backends) {
    // linked by hand: the guest reads the origin from `omnia:config`
    let status = deployment
        .run(backends, |deployment| {
            deployment.host::<WasiHttp>()?;
            deployment.host::<WasiConfig>()?;
            deployment.host::<WasiOtel>()?;
            Ok(())
        })
        .await
        .expect("guest runs");
    assert_eq!(status, ExitStatus::SUCCESS, "the guest's assertions held");
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

// The origin listens on `127.0.0.1`, so one origin answers both names the
// guest uses, and a refusal shows as a hit that never arrives.
#[tokio::test]
async fn http_outgoing_hosts() {
    let (origin, url) = Origin::serve().await;
    let (_, port) = url.rsplit_once(':').expect("the origin's port");
    let backends = Backends::defaults().await.config([("PORT", port.to_owned())]);
    let grants = Grants::none().hosts(["localhost"]);

    let guest =
        GuestEntry::new("guest", test_programs::HTTP_OUTGOING_HOSTS).command().grants(grants);
    run_deployment(Deployment::new().entry(guest), backends).await;

    // the address and the hop to it were refused before any connection
    let hits = origin.hits();
    let paths: Vec<_> = hits.iter().map(|hit| hit.path.as_str()).collect();
    assert_eq!(paths, ["/resource", "/redirect", "/resource", "/redirect"]);
}

// rcgen's defaults: no EKU, no key usage, no basic constraints
fn cert_params(configure: impl FnOnce(&mut CertificateParams)) -> CertificateParams {
    let mut params = CertificateParams::new(Vec::<String>::new()).expect("parameters");
    configure(&mut params);
    params
}

fn client_auth(params: &mut CertificateParams) {
    params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ClientAuth];
    params.key_usages = vec![KeyUsagePurpose::DigitalSignature];
}

// a throwaway self-signed certificate then its PKCS#8 key, minted per run so
// no private key is ever checked in
fn client_cert(configure: impl FnOnce(&mut CertificateParams)) -> String {
    let key = KeyPair::generate().expect("key pair");
    let certificate = cert_params(configure).self_signed(&key).expect("certificate");
    Base64::encode_string(format!("{}{}", certificate.pem(), key.serialize_pem()).as_bytes())
}

// a client-auth leaf, then the CA that signed it, then the leaf's key; the CA
// lacks `clientAuth`, which only matters if it is mistaken for the identity
fn client_cert_chain() -> String {
    let ca_key = KeyPair::generate().expect("CA key pair");
    let ca_params = cert_params(|params| {
        params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
        params.key_usages = vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::CrlSign];
    });
    let ca = ca_params.self_signed(&ca_key).expect("CA certificate");

    let leaf_key = KeyPair::generate().expect("leaf key pair");
    let leaf = cert_params(client_auth)
        .signed_by(&leaf_key, &Issuer::from_params(&ca_params, &ca_key))
        .expect("leaf certificate");

    let bundle = format!("{}{}{}", leaf.pem(), ca.pem(), leaf_key.serialize_pem());
    Base64::encode_string(bundle.as_bytes())
}

#[tokio::test]
async fn http_outgoing_client_cert() {
    let (origin, url) = Origin::serve().await;
    let backends = Backends::defaults().await.config([
        ("ORIGIN", url),
        ("CLIENT_CERT_OK", client_cert(client_auth)),
        ("CLIENT_CERT_NO_EKU", client_cert(|_| {})),
        ("CLIENT_CERT_CHAIN", client_cert_chain()),
        (
            "CLIENT_CERT_SERVER",
            client_cert(|params| {
                params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
                params.key_usages = vec![KeyUsagePurpose::DigitalSignature];
            }),
        ),
        (
            "CLIENT_CERT_CA",
            client_cert(|params| params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained)),
        ),
        (
            "CLIENT_CERT_KEY_ENCIPHERMENT",
            client_cert(|params| params.key_usages = vec![KeyUsagePurpose::KeyEncipherment]),
        ),
        (
            "CLIENT_CERT_EXPIRED",
            client_cert(|params| {
                params.not_before = date_time_ymd(2020, 1, 1);
                params.not_after = date_time_ymd(2021, 1, 1);
            }),
        ),
        (
            "CLIENT_CERT_NOT_YET_VALID",
            client_cert(|params| {
                params.not_before = date_time_ymd(2100, 1, 1);
                params.not_after = date_time_ymd(2101, 1, 1);
            }),
        ),
    ]);

    run_guest(test_programs::HTTP_OUTGOING_CLIENT_CERT, backends).await;

    // only the accepted bundles reached the origin, and none left the host
    let hits = origin.hits();
    let paths: Vec<_> = hits.iter().map(|hit| hit.path.as_str()).collect();
    assert_eq!(paths, ["/mtls", "/no-eku", "/chain"]);
    for hit in &hits {
        assert!(hit.headers.get("client-cert").is_none(), "Client-Cert header stripped");
    }
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
async fn http_builder() {
    let (origin, url) = Origin::serve().await;
    let (stalling, stalling_url) = StallingOrigin::serve().await;
    let backends = Backends::defaults().await.config([("ORIGIN", url), ("STALLING", stalling_url)]);

    run_guest(test_programs::HTTP_BUILDER, backends).await;

    // the post arrived as built: its method, credential, content type, and body
    let hits = origin.hits();
    let paths: Vec<_> = hits.iter().map(|hit| hit.path.as_str()).collect();
    assert_eq!(paths, ["/orders", "/resource"]);
    let posted = &hits[0];
    assert_eq!(posted.method, Method::POST);
    assert_eq!(posted.headers[AUTHORIZATION], "Bearer t0ken");
    assert_eq!(posted.headers[CONTENT_TYPE], "application/json");
    let body: Value = serde_json::from_slice(&posted.body).expect("a JSON body");
    assert_eq!(body, json!({ "sku": "a-1", "quantity": 2 }));

    // the timeout fired on a connection the origin held, not before one was made
    assert_eq!(stalling.accepted(), 1);
}

#[tokio::test]
async fn http_incoming_echo() {
    const GUEST_TIMEOUT: Duration = Duration::from_secs(30);

    // no `wasi:cli/run` to drive: boot, then dispatch through the in-process handler
    let guest = std::fs::read(test_programs::HTTP_INCOMING_ECHO).expect("guest bytes");
    // declared from a path that does not exist, so it loads (and fails) at first use
    let absent = std::env::temp_dir().join("omnia-http-absent-guest.wasm");
    let runtime = Deployment::new()
        .entry(GuestEntry::new("guest", guest).route_http("/echo"))
        .entry(GuestEntry::new("absent", absent).route_http("/echo/absent"))
        .guest_timeout(GUEST_TIMEOUT)
        .boot(Backends::defaults().await, |deployment| {
            deployment.host::<WasiHttp>()?;
            deployment.host::<WasiOtel>()?;
            Ok(())
        })
        .await
        .expect("runtime boots");
    let handler = HttpHandler::new(&runtime)
        .expect("http routes consistent")
        .expect("the guest exports the http handler");

    // get: the message is the query parameter; the request id comes back from
    // the metadata, and the host's deadline replaces the one the client sent
    let request = Request::get("/echo?message=hi")
        .header(HOST, "echo.test")
        .header("x-request-id", "req-get")
        .header("x-deadline", "1")
        .body(Full::new(Bytes::new()))
        .expect("request");
    let earliest = epoch_millis(SystemTime::now() + GUEST_TIMEOUT);
    let response = handler.handle(request).await.expect("handled");
    let latest = epoch_millis(SystemTime::now() + GUEST_TIMEOUT);
    assert_eq!(response.status(), StatusCode::OK);
    let (echo, deadline) = echoed(response).await;
    assert_eq!(echo, json!({ "message": "hi", "request_id": "req-get" }));
    assert!(
        deadline.is_some_and(|deadline| (earliest..=latest).contains(&deadline)),
        "{deadline:?}"
    );

    // post: the message is the json body
    let request = Request::post("/echo")
        .header(HOST, "echo.test")
        .header(CONTENT_TYPE, "application/json")
        .header("x-request-id", "req-post")
        .body(Full::new(Bytes::from_static(br#"{"message":"posted"}"#)))
        .expect("request");
    let response = handler.handle(request).await.expect("handled");
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(echoed(response).await.0, json!({ "message": "posted", "request_id": "req-post" }));

    // http/2: authority and scheme arrive on the request URI with no `Host` header
    let request = Request::get("http://echo.test/echo?message=h2")
        .header("x-request-id", "req-h2")
        .body(Full::new(Bytes::new()))
        .expect("request");
    let response = handler.handle(request).await.expect("handled");
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(echoed(response).await.0, json!({ "message": "h2", "request_id": "req-h2" }));

    // the declared routes drive the trigger: the longest prefix wins, matched
    // by whole segment, so `/echo/absentee` runs the echo guest (which has no
    // such route) and a path beside both is a miss that runs nothing
    for (path, status, ran) in [
        ("/echo/absent", StatusCode::INTERNAL_SERVER_ERROR, false),
        ("/echo/absent/x", StatusCode::INTERNAL_SERVER_ERROR, false),
        ("/echo/absentee", StatusCode::NOT_FOUND, true),
        ("/other", StatusCode::NOT_FOUND, false),
    ] {
        let built = runtime.stores_built();
        let request = Request::get(path)
            .header(HOST, "echo.test")
            .body(Full::new(Bytes::new()))
            .expect("request");
        let response = handler.handle(request).await.expect("handled");
        assert_eq!(response.status(), status, "{path}");
        assert_eq!(runtime.stores_built() > built, ran, "{path}");
    }

    runtime.shutdown();
}

// A request holds its gate slot until its guest is done with it, and the echo
// guest is not done until the request body ends. With a gate of one, a
// request that may not wait is refused while the slot is held, and one that
// may wait is served once it frees.
#[tokio::test]
async fn full_gate() {
    let runtime = gate_of_one().await;
    let handler = |queue_wait| {
        let options = ServerOptions {
            queue_wait,
            ..ServerOptions::default()
        };
        HttpHandler::with_options(&runtime, options)
            .expect("http routes consistent")
            .expect("the guest exports the http handler")
    };
    let refusing = handler(Duration::ZERO);
    let waiting = handler(Duration::from_secs(30));

    let (body, request) = held_post();
    let first = tokio::spawn({
        let handler = refusing.clone();
        async move { handler.handle(request).await }
    });
    until(|| runtime.invocations().in_flight == 1).await;

    // refused at once, without running a guest
    let built = runtime.stores_built();
    let response = refusing.handle(echo_get("busy")).await.expect("handled");
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(response.headers()[RETRY_AFTER], "1");
    assert_eq!(runtime.stores_built(), built);

    // queued behind the first, then served once its slot frees
    let third = tokio::spawn(async move { waiting.handle(echo_get("waited")).await });
    until(|| runtime.invocations().waiting == 1).await;
    body.unbounded_send(Bytes::from_static(br#""held"}"#)).expect("the body is open");
    drop(body);

    let response = first.await.expect("first request joins").expect("handled");
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(echoed(response).await.0["message"], "held");
    let response = third.await.expect("third request joins").expect("handled");
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(echoed(response).await.0["message"], "waited");

    // the slot frees as the guest's task drops its store, after the body
    until(|| runtime.invocations().in_flight == 0).await;

    runtime.shutdown();
}

// The health path enters no guest, so it takes no slot: it is `200` while a
// held request fills a gate of one, though the guest's route covers it and
// the probe names no host.
#[tokio::test]
async fn health_path() {
    let runtime = gate_of_one().await;
    let options = ServerOptions {
        health_path: Some("/echo/health".to_owned()),
        ..ServerOptions::default()
    };
    let handler = HttpHandler::with_options(&runtime, options)
        .expect("http routes consistent")
        .expect("the guest exports the http handler");

    let (body, request) = held_post();
    let held = tokio::spawn({
        let handler = handler.clone();
        async move { handler.handle(request).await }
    });
    until(|| runtime.invocations().in_flight == 1).await;

    let built = runtime.stores_built();
    let probe = Request::get("/echo/health").body(Full::new(Bytes::new())).expect("request");
    let response = handler.handle(probe).await.expect("handled");
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(runtime.stores_built(), built);

    body.unbounded_send(Bytes::from_static(br#""held"}"#)).expect("the body is open");
    drop(body);
    held.await.expect("held request joins").expect("handled");

    runtime.shutdown();
}

// the echo guest at `/echo` behind a gate of one
async fn gate_of_one() -> Runtime<Backends> {
    let guest = std::fs::read(test_programs::HTTP_INCOMING_ECHO).expect("guest bytes");
    let mut options = RuntimeOptions::load_env().expect("runtime options load");
    options.max_concurrent_invocations = Some(1);
    Deployment::new()
        .entry(GuestEntry::new("guest", guest).route_http("/echo"))
        .options(options)
        .boot(Backends::defaults().await, |deployment| {
            deployment.host::<WasiHttp>()?;
            deployment.host::<WasiOtel>()?;
            Ok(())
        })
        .await
        .expect("runtime boots")
}

// a POST to the echo guest sent only as far as `{"message":`, so its guest
// holds its slot until the rest of the body is sent or the sender drops
fn held_post()
-> (UnboundedSender<Bytes>, Request<impl Body<Data = Bytes, Error = Infallible> + Send + 'static>) {
    let (body, chunks) = futures::channel::mpsc::unbounded::<Bytes>();
    body.unbounded_send(Bytes::from_static(br#"{"message":"#)).expect("the body is open");
    let request = Request::post("/echo")
        .header(HOST, "echo.test")
        .header(CONTENT_TYPE, "application/json")
        .body(StreamBody::new(chunks.map(|chunk| Ok::<_, Infallible>(Frame::data(chunk)))))
        .expect("request");
    (body, request)
}

fn echo_get(message: &str) -> Request<Full<Bytes>> {
    Request::get(format!("/echo?message={message}"))
        .header(HOST, "echo.test")
        .body(Full::new(Bytes::new()))
        .expect("request")
}

// polls `ready` until it holds, failing the test when it never does
async fn until(ready: impl Fn() -> bool + Send + Sync) {
    tokio::time::timeout(Duration::from_secs(30), async {
        while !ready() {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("the runtime reaches the awaited state");
}

// An MCP tool awaits a capability before it answers, served through the
// guest's `wasi:http` handler.
#[tokio::test]
async fn http_mcp_server() {
    let guest = std::fs::read(test_programs::HTTP_MCP_SERVER).expect("guest bytes");
    let backends = Backends::defaults().await.config([("GREETING", "hello")]);
    let runtime = Deployment::new()
        .entry(GuestEntry::new("guest", guest).route_http("/mcp"))
        .boot(backends, |deployment| {
            deployment.host::<WasiHttp>()?;
            deployment.host::<WasiConfig>()?;
            deployment.host::<WasiOtel>()?;
            Ok(())
        })
        .await
        .expect("runtime boots");
    let handler = HttpHandler::new(&runtime)
        .expect("http routes consistent")
        .expect("the guest exports the http handler");

    let call = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "tools/call",
        "params": { "name": "greet", "arguments": { "name": "ann" } },
    });
    let request = Request::post("/mcp")
        .header(HOST, "mcp.test")
        .header(CONTENT_TYPE, "application/json")
        .body(Full::new(Bytes::from(call.to_string())))
        .expect("request");
    let response = handler.handle(request).await.expect("handled");

    assert_eq!(response.status(), StatusCode::OK);
    let reply = collect(response).await;
    assert_eq!(reply["result"]["isError"], false, "{reply}");
    assert_eq!(reply["result"]["content"][0]["text"], "hello, ann", "{reply}");

    runtime.shutdown();
}

async fn collect(response: Response<omnia_wasi_http::OutgoingBody>) -> Value {
    let body = response.into_body().collect().await.expect("body streams").to_bytes();
    serde_json::from_slice(&body).expect("JSON body")
}

// the echo with its deadline split out, since the deadline differs per request
async fn echoed(response: Response<omnia_wasi_http::OutgoingBody>) -> (Value, Option<u64>) {
    let mut echo = collect(response).await;
    let deadline = echo.as_object_mut().and_then(|fields| fields.remove("deadline"));
    (echo, deadline.as_ref().and_then(Value::as_u64))
}

fn epoch_millis(instant: SystemTime) -> u64 {
    let since_epoch = instant.duration_since(UNIX_EPOCH).expect("after the epoch");
    u64::try_from(since_epoch.as_millis()).expect("fits in u64 milliseconds")
}
